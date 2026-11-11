//! Rotas de administração (`/admin`), exigem login com perfil de admin.

use crate::auth::ip::chave_ip;
use crate::auth::middleware::{exigir_admin, exigir_login, UsuarioAutenticado};
use crate::auth::rotas::validar_senha;
use crate::auth::{penalidade, senha};
use crate::error::AppError;
use crate::estado::AppState;
use crate::extract::{Caminho, Consulta, Corpo};
use crate::listagem::{self, Pagina, ParametrosLista, Tabela};
use crate::validacao::Validador;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, MySqlPool};
use std::net::IpAddr;

const TABELA: Tabela = Tabela {
    nome: "USUARIOS",
    colunas: "id, login, nome, admin, ativo, created_at, updated_at",
    busca: &["login", "nome"],
    ordenaveis: &[
        ("id", "id"),
        ("login", "login"),
        ("nome", "nome"),
        ("created_at", "created_at"),
    ],
};

#[derive(Debug, Serialize, FromRow)]
pub struct UsuarioAdmin {
    pub id: u64,
    pub login: String,
    pub nome: String,
    pub admin: bool,
    pub ativo: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AtualizarUsuario {
    pub nome: Option<String>,
    pub admin: Option<bool>,
    pub ativo: Option<bool>,
    /// Redefine a senha (ex.: a pedido da pessoa). Encerra todas as sessões dela.
    pub senha: Option<String>,
}

pub fn router(estado: AppState) -> Router<AppState> {
    Router::new()
        .route("/usuarios", get(listar))
        .route("/usuarios/{id}", get(obter).put(atualizar))
        .route("/usuarios/{id}/desbloquear", post(desbloquear))
        .route("/penalidades/ip/{ip}", delete(desbloquear_ip))
        // A última camada adicionada roda primeiro: login, depois admin.
        .route_layer(from_fn(exigir_admin))
        .route_layer(from_fn_with_state(estado, exigir_login))
}

async fn buscar(pool: &MySqlPool, id: u64) -> Result<UsuarioAdmin, AppError> {
    sqlx::query_as::<_, UsuarioAdmin>(
        "SELECT id, login, nome, admin, ativo, created_at, updated_at FROM USUARIOS WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("USUARIOS id={id} não encontrado")))
}

async fn listar(
    State(pool): State<MySqlPool>,
    Consulta(params): Consulta<ParametrosLista>,
) -> Result<Json<Pagina<UsuarioAdmin>>, AppError> {
    Ok(Json(listagem::listar(&pool, &TABELA, &params, &[]).await?))
}

async fn obter(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
) -> Result<Json<UsuarioAdmin>, AppError> {
    Ok(Json(buscar(&pool, id).await?))
}

async fn atualizar(
    State(pool): State<MySqlPool>,
    Extension(admin): Extension<UsuarioAutenticado>,
    Caminho(id): Caminho<u64>,
    Corpo(req): Corpo<AtualizarUsuario>,
) -> Result<Json<UsuarioAdmin>, AppError> {
    let atual = buscar(&pool, id).await?;

    let mut v = Validador::new();
    v.algum_campo(&[
        req.nome.is_some(),
        req.admin.is_some(),
        req.ativo.is_some(),
        req.senha.is_some(),
    ]);
    v.texto_obrigatorio_se_enviado("nome", req.nome.as_deref(), 255);
    if let Some(s) = &req.senha {
        validar_senha(&mut v, s, &atual.login);
    }
    // Evita que o admin se tranque fora do sistema por engano.
    if id == admin.id && (req.admin == Some(false) || req.ativo == Some(false)) {
        v.erro(
            "admin",
            "você não pode remover o próprio perfil de admin nem se desativar",
        );
    }
    v.finalizar()?;

    let hash = match req.senha {
        Some(s) => Some(senha::gerar_hash(s).await?),
        None => None,
    };

    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE USUARIOS SET nome = COALESCE(?, nome), admin = COALESCE(?, admin), \
         ativo = COALESCE(?, ativo), senha_hash = COALESCE(?, senha_hash) WHERE id = ?",
    )
    .bind(&req.nome)
    .bind(req.admin)
    .bind(req.ativo)
    .bind(&hash)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if hash.is_some() || req.ativo == Some(false) {
        sqlx::query("DELETE FROM SESSOES_AUTH WHERE usuario_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    tracing::info!(
        "Admin '{}' alterou o usuário '{}' (senha redefinida: {})",
        admin.login,
        atual.login,
        hash.is_some()
    );
    Ok(Json(buscar(&pool, id).await?))
}

/// Zera a penalidade da conta e de todos os dispositivos do usuário.
async fn desbloquear(
    State(pool): State<MySqlPool>,
    Extension(admin): Extension<UsuarioAutenticado>,
    Caminho(id): Caminho<u64>,
) -> Result<StatusCode, AppError> {
    let usuario = buscar(&pool, id).await?;
    penalidade::limpar_usuario(&pool, usuario.id, &usuario.login).await?;
    tracing::info!(
        "Admin '{}' desbloqueou a conta '{}'",
        admin.login,
        usuario.login
    );
    Ok(StatusCode::NO_CONTENT)
}

/// Zera a penalidade de um IP (login e cadastro).
async fn desbloquear_ip(
    State(pool): State<MySqlPool>,
    Extension(admin): Extension<UsuarioAutenticado>,
    Caminho(ip): Caminho<IpAddr>,
) -> Result<StatusCode, AppError> {
    penalidade::limpar(&pool, &chave_ip("ip", ip)).await?;
    penalidade::limpar(&pool, &chave_ip("cadastro", ip)).await?;
    tracing::info!("Admin '{}' desbloqueou o IP {ip}", admin.login);
    Ok(StatusCode::NO_CONTENT)
}
