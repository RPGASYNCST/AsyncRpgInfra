use crate::error::AppError;
use crate::extract::{Caminho, Consulta, Corpo};
use crate::listagem::{self, Filtro, Pagina, ParametrosLista, Tabela, Valor};
use crate::validacao::{Validador, TEXT_MAX_BYTES};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, MySqlPool};

const TABELA: Tabela = Tabela {
    nome: "SESSAO_PLAYERS",
    colunas: "id, sessao_id, name, player_name, resume, created_at, updated_at",
    busca: &["name", "player_name", "resume"],
    ordenaveis: &[
        ("id", "id"),
        ("sessao_id", "sessao_id"),
        ("name", "name"),
        ("player_name", "player_name"),
        ("created_at", "created_at"),
        ("updated_at", "updated_at"),
    ],
};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SessaoPlayer {
    pub id: u64,
    pub sessao_id: u64,
    pub name: String,
    pub player_name: String,
    pub resume: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct FiltroSessaoPlayers {
    pub sessao_id: Option<u64>,
    pub player_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessaoPlayer {
    pub sessao_id: u64,
    pub name: String,
    pub player_name: String,
    pub resume: Option<String>,
}

impl CreateSessaoPlayer {
    fn validar(&self) -> Result<(), AppError> {
        let mut v = Validador::new();
        v.id("sessao_id", self.sessao_id);
        v.texto_obrigatorio("name", &self.name, 255);
        v.texto_obrigatorio("player_name", &self.player_name, 255);
        v.texto_opcional("resume", self.resume.as_deref(), TEXT_MAX_BYTES);
        v.finalizar()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessaoPlayer {
    pub sessao_id: Option<u64>,
    pub name: Option<String>,
    pub player_name: Option<String>,
    pub resume: Option<String>,
}

impl UpdateSessaoPlayer {
    fn validar(&self) -> Result<(), AppError> {
        let mut v = Validador::new();
        v.algum_campo(&[
            self.sessao_id.is_some(),
            self.name.is_some(),
            self.player_name.is_some(),
            self.resume.is_some(),
        ]);
        v.id_se_enviado("sessao_id", self.sessao_id);
        v.texto_obrigatorio_se_enviado("name", self.name.as_deref(), 255);
        v.texto_obrigatorio_se_enviado("player_name", self.player_name.as_deref(), 255);
        v.texto_opcional("resume", self.resume.as_deref(), TEXT_MAX_BYTES);
        v.finalizar()
    }
}

pub fn router() -> Router<MySqlPool> {
    Router::new()
        .route("/", get(list_all).post(create))
        .route("/{id}", get(get_by_id).put(update).delete(delete))
}

async fn list_all(
    State(pool): State<MySqlPool>,
    Consulta(params): Consulta<ParametrosLista>,
    Consulta(filtro): Consulta<FiltroSessaoPlayers>,
) -> Result<Json<Pagina<SessaoPlayer>>, AppError> {
    let mut filtros: Vec<Filtro> = Vec::new();
    if let Some(sessao_id) = filtro.sessao_id {
        filtros.push(("sessao_id", Valor::Id(sessao_id)));
    }
    if let Some(player_name) = filtro.player_name {
        filtros.push(("player_name", Valor::Texto(player_name)));
    }
    Ok(Json(
        listagem::listar(&pool, &TABELA, &params, &filtros).await?,
    ))
}

async fn buscar(pool: &MySqlPool, id: u64) -> Result<SessaoPlayer, AppError> {
    sqlx::query_as::<_, SessaoPlayer>(
        "SELECT id, sessao_id, name, player_name, resume, created_at, updated_at \
         FROM SESSAO_PLAYERS WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("SESSAO_PLAYERS id={id} não encontrado")))
}

async fn get_by_id(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
) -> Result<Json<SessaoPlayer>, AppError> {
    Ok(Json(buscar(&pool, id).await?))
}

async fn create(
    State(pool): State<MySqlPool>,
    Corpo(payload): Corpo<CreateSessaoPlayer>,
) -> Result<(StatusCode, Json<SessaoPlayer>), AppError> {
    payload.validar()?;

    let result = sqlx::query(
        "INSERT INTO SESSAO_PLAYERS (sessao_id, name, player_name, resume) VALUES (?, ?, ?, ?)",
    )
    .bind(payload.sessao_id)
    .bind(&payload.name)
    .bind(&payload.player_name)
    .bind(&payload.resume)
    .execute(&pool)
    .await?;

    let created = buscar(&pool, result.last_insert_id()).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
    Corpo(payload): Corpo<UpdateSessaoPlayer>,
) -> Result<Json<SessaoPlayer>, AppError> {
    payload.validar()?;
    let current = buscar(&pool, id).await?;

    let sessao_id = payload.sessao_id.unwrap_or(current.sessao_id);
    let name = payload.name.unwrap_or(current.name);
    let player_name = payload.player_name.unwrap_or(current.player_name);
    let resume = payload.resume.or(current.resume);

    sqlx::query(
        "UPDATE SESSAO_PLAYERS SET sessao_id = ?, name = ?, player_name = ?, resume = ? WHERE id = ?",
    )
    .bind(sessao_id)
    .bind(name)
    .bind(player_name)
    .bind(resume)
    .bind(id)
    .execute(&pool)
    .await?;

    Ok(Json(buscar(&pool, id).await?))
}

async fn delete(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
) -> Result<StatusCode, AppError> {
    let result = sqlx::query("DELETE FROM SESSAO_PLAYERS WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "SESSAO_PLAYERS id={id} não encontrado"
        )));
    }

    Ok(StatusCode::NO_CONTENT)
}
