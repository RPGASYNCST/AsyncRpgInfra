//! Rotas públicas de autenticação (`/auth`).

use crate::auth::ip::{chave_ip, ip_cliente};
use crate::auth::middleware::{exigir_login, token_bearer, UsuarioAutenticado};
use crate::auth::penalidade::{self, chave_conta, chave_dispositivo, segundos_ate};
use crate::auth::{senha, token};
use crate::error::AppError;
use crate::estado::AppState;
use crate::extract::Corpo;
use crate::validacao::Validador;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

pub const LOGIN_MIN: usize = 3;
pub const LOGIN_MAX: usize = 50;
pub const SENHA_MIN: usize = 6;
/// Teto da senha: Argon2 com entradas enormes vira vetor de negação de serviço.
pub const SENHA_MAX: usize = 128;
/// Cadastros permitidos por IP antes de a espera exponencial começar.
pub const CADASTROS_LIVRES_POR_IP: u32 = 2;

#[derive(Debug, Deserialize)]
pub struct CadastroRequest {
    pub login: String,
    pub nome: String,
    pub senha: String,
    pub sessao_id: Option<u64>,
    pub codigo_convite: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub login: String,
    pub senha: String,
    /// Token de dispositivo confiável recebido em um login anterior neste computador.
    pub dispositivo: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UsuarioPublico {
    pub id: u64,
    pub login: String,
    pub nome: String,
    pub admin: bool,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// Enviar em `Authorization: Bearer <token>` nas rotas `/api`.
    pub token: String,
    pub expira_em: DateTime<Utc>,
    /// Novo token de dispositivo confiável; `null` quando o enviado continua válido.
    pub dispositivo: Option<String>,
    pub usuario: UsuarioPublico,
    pub sessao_id: u64,
    pub sessao_nome: String,
    pub sessao_sistema: String,
    pub players_id: u64,
    pub tem_ficha: bool,
}

pub fn router(estado: AppState) -> Router<AppState> {
    let protegidas = Router::new()
        .route("/logout", post(logout))
        .route("/eu", get(eu))
        .route_layer(from_fn_with_state(estado, exigir_login));

    Router::new()
        .route("/cadastro", post(cadastro))
        .route("/login", post(login))
        .merge(protegidas)
}

pub fn validar_login(v: &mut Validador, login: &str) {
    let tamanho = login.chars().count();
    if !(LOGIN_MIN..=LOGIN_MAX).contains(&tamanho) {
        v.erro(
            "login",
            format!("deve ter entre {LOGIN_MIN} e {LOGIN_MAX} caracteres"),
        );
    } else if !login
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        || !login.starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        v.erro(
            "login",
            "use apenas letras minúsculas sem acento, números, '.', '_' e '-', começando com letra ou número",
        );
    }
}

pub fn validar_senha(v: &mut Validador, senha: &str, login: &str) {
    let tamanho = senha.chars().count();
    if !(SENHA_MIN..=SENHA_MAX).contains(&tamanho) {
        v.erro(
            "senha",
            format!("deve ter entre {SENHA_MIN} e {SENHA_MAX} caracteres"),
        );
    } else if senha.to_lowercase() == login {
        v.erro("senha", "não pode ser igual ao login");
    }
}

fn normalizar_login(login: &str) -> String {
    login.trim().to_lowercase()
}

async fn cadastro(
    State(estado): State<AppState>,
    ConnectInfo(par): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Corpo(req): Corpo<CadastroRequest>,
) -> Result<(StatusCode, Json<UsuarioPublico>), AppError> {
    let ip = ip_cliente(par.ip(), &headers, &estado.auth.proxies_confiaveis);
    let chave_cadastro = chave_ip("cadastro", ip);
    if let Some(ate) = penalidade::bloqueio(&estado.pool, &chave_cadastro).await? {
        return Err(AppError::MuitasTentativas {
            aguarde_segundos: segundos_ate(ate),
        });
    }

    let login = normalizar_login(&req.login);
    let mut v = Validador::new();
    validar_login(&mut v, &login);
    v.texto_obrigatorio("nome", &req.nome, 255);
    validar_senha(&mut v, &req.senha, &login);
    v.finalizar()?;

    let hash = senha::gerar_hash(req.senha).await?;
    let resultado = sqlx::query("INSERT INTO USUARIOS (login, nome, senha_hash) VALUES (?, ?, ?)")
        .bind(&login)
        .bind(&req.nome)
        .bind(hash)
        .execute(&estado.pool)
        .await;
    let id = match resultado {
        Ok(r) => r.last_insert_id(),
        Err(err) => {
            return Err(match AppError::from(err) {
                AppError::Conflict(_) => AppError::Conflict("Este login já está em uso.".into()),
                outro => outro,
            })
        }
    };

    let mut sessao_resolvida = req.sessao_id;
    if sessao_resolvida.is_none() {
        if let Some(convite) = req
            .codigo_convite
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Ok(id_num) = convite.parse::<u64>() {
                let existe: Option<(u64,)> =
                    sqlx::query_as("SELECT id FROM SESSAO_RPG WHERE id = ? LIMIT 1")
                        .bind(id_num)
                        .fetch_optional(&estado.pool)
                        .await?;
                sessao_resolvida = existe.map(|(sid,)| sid);
            }
            if sessao_resolvida.is_none() {
                let por_nome: Option<(u64,)> = sqlx::query_as(
                    "SELECT id FROM SESSAO_RPG WHERE name = ? ORDER BY id DESC LIMIT 1",
                )
                .bind(convite)
                .fetch_optional(&estado.pool)
                .await?;
                sessao_resolvida = por_nome.map(|(sid,)| sid);
            }
        }
    }

    if let Some(sid) = sessao_resolvida {
        let _ = sqlx::query(
            "INSERT INTO SESSAO_PLAYERS (sessao_id, name, player_name, resume) VALUES (?, ?, ?, ?)",
        )
        .bind(sid)
        .bind(&req.nome)
        .bind(&login)
        .bind("Jogador cadastrado diretamente via código de convite")
        .execute(&estado.pool)
        .await;
    }

    penalidade::registrar(&estado.pool, &chave_cadastro, CADASTROS_LIVRES_POR_IP).await?;
    tracing::info!("Novo usuário cadastrado: '{login}' (id={id}) a partir de {ip}");

    Ok((
        StatusCode::CREATED,
        Json(UsuarioPublico {
            id,
            login,
            nome: req.nome,
            admin: false,
        }),
    ))
}

/// Dispositivo confiável do usuário, se o token enviado for dele.
async fn dispositivo_confiavel(
    estado: &AppState,
    usuario_id: u64,
    token_dispositivo: Option<&str>,
) -> Result<Option<u64>, AppError> {
    let Some(t) = token_dispositivo.filter(|t| token::formato_valido(t)) else {
        return Ok(None);
    };
    let linha: Option<(u64,)> =
        sqlx::query_as("SELECT id FROM DISPOSITIVOS WHERE token_hash = ? AND usuario_id = ?")
            .bind(token::hash(t))
            .bind(usuario_id)
            .fetch_optional(&estado.pool)
            .await?;
    Ok(linha.map(|(id,)| id))
}

async fn login(
    State(estado): State<AppState>,
    ConnectInfo(par): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Corpo(req): Corpo<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let pool = &estado.pool;
    let ip = ip_cliente(par.ip(), &headers, &estado.auth.proxies_confiaveis);
    let login = normalizar_login(&req.login);

    // Entrada absurda nem chega ao Argon2 e não conta como tentativa.
    if login.is_empty()
        || login.chars().count() > LOGIN_MAX
        || req.senha.chars().count() > SENHA_MAX
    {
        return Err(AppError::LoginRecusado {
            aguarde_segundos: 0,
        });
    }

    let usuario: Option<(u64, String, String, bool, bool)> =
        sqlx::query_as("SELECT id, nome, senha_hash, admin, ativo FROM USUARIOS WHERE login = ?")
            .bind(&login)
            .fetch_optional(pool)
            .await?;

    let dispositivo = match &usuario {
        Some((id, ..)) => dispositivo_confiavel(&estado, *id, req.dispositivo.as_deref()).await?,
        None => None,
    };

    // Dispositivo confiável: só a penalidade dele. Desconhecido: conta + IP.
    let chaves = match dispositivo {
        Some(id) => vec![chave_dispositivo(id)],
        None => vec![chave_conta(&login), chave_ip("ip", ip)],
    };

    if let Some(ate) = penalidade::maior_bloqueio(pool, &chaves).await? {
        return Err(AppError::MuitasTentativas {
            aguarde_segundos: segundos_ate(ate),
        });
    }

    let hash = usuario.as_ref().map(|(_, _, h, _, _)| h.clone());
    let senha_ok = senha::verificar(req.senha, hash).await?;
    let ativo = usuario.as_ref().is_some_and(|(_, _, _, _, ativo)| *ativo);

    let Some((usuario_id, nome, _, admin, _)) = usuario.filter(|_| senha_ok && ativo) else {
        let mut maior: Option<DateTime<Utc>> = None;
        for chave in &chaves {
            if let Some(ate) = penalidade::registrar(pool, chave, 0).await? {
                maior = Some(maior.map_or(ate, |m| m.max(ate)));
            }
        }
        tracing::warn!("Login recusado para '{login}' a partir de {ip}");
        return Err(AppError::LoginRecusado {
            aguarde_segundos: maior.map_or(0, segundos_ate),
        });
    };

    // Sucesso: zera conta e dispositivo (o IP não, ver `penalidade.rs`).
    penalidade::limpar(pool, &chave_conta(&login)).await?;
    let novo_dispositivo = match dispositivo {
        Some(id) => {
            penalidade::limpar(pool, &chave_dispositivo(id)).await?;
            sqlx::query("UPDATE DISPOSITIVOS SET ultimo_uso_em = ? WHERE id = ?")
                .bind(Utc::now())
                .bind(id)
                .execute(pool)
                .await?;
            None
        }
        None => {
            let t = token::gerar()?;
            sqlx::query("INSERT INTO DISPOSITIVOS (usuario_id, token_hash) VALUES (?, ?)")
                .bind(usuario_id)
                .bind(token::hash(&t))
                .execute(pool)
                .await?;
            Some(t)
        }
    };

    let token_sessao = token::gerar()?;
    let expira_em = Utc::now() + estado.auth.duracao_sessao;
    sqlx::query("INSERT INTO SESSOES_AUTH (token_hash, usuario_id, expira_em) VALUES (?, ?, ?)")
        .bind(token::hash(&token_sessao))
        .bind(usuario_id)
        .bind(expira_em)
        .execute(pool)
        .await?;

    let vinculo_existente: Option<(u64, u64)> = sqlx::query_as(
        "SELECT id, sessao_id FROM SESSAO_PLAYERS WHERE player_name = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(&login)
    .fetch_optional(pool)
    .await?;

    let (players_id, sessao_id) = match vinculo_existente {
        Some(par) => par,
        None => {
            let sessao_padrao: Option<(u64,)> =
                sqlx::query_as("SELECT id FROM SESSAO_RPG ORDER BY id DESC LIMIT 1")
                    .fetch_optional(pool)
                    .await?;
            let sid = match sessao_padrao {
                Some((id_s,)) => id_s,
                None => sqlx::query(
                    "INSERT INTO SESSAO_RPG (name, description, `system`) VALUES (?, ?, ?)",
                )
                .bind("Crônicas de Kharos")
                .bind("Campanha principal")
                .bind("D&D")
                .execute(pool)
                .await?
                .last_insert_id(),
            };
            let pid = sqlx::query(
                "INSERT INTO SESSAO_PLAYERS (sessao_id, name, player_name, resume) VALUES (?, ?, ?, ?)",
            )
            .bind(sid)
            .bind(&nome)
            .bind(&login)
            .bind("Operador vinculado no primeiro acesso")
            .execute(pool)
            .await?
            .last_insert_id();
            (pid, sid)
        }
    };

    let (sessao_nome, sessao_sistema): (String, String) =
        sqlx::query_as("SELECT name, `system` FROM SESSAO_RPG WHERE id = ?")
            .bind(sessao_id)
            .fetch_optional(pool)
            .await?
            .unwrap_or_else(|| ("Crônicas de Kharos".to_string(), "D&D".to_string()));

    let ficha_existente: Option<(u64,)> = sqlx::query_as(
        "SELECT id FROM PLAYERS_SHEETS WHERE sessao_id = ? AND players_id = ? LIMIT 1",
    )
    .bind(sessao_id)
    .bind(players_id)
    .fetch_optional(pool)
    .await?;

    let tem_ficha = ficha_existente.is_some();

    tracing::info!(
        "Login de '{login}' a partir de {ip} (sessao_id={sessao_id}, tem_ficha={tem_ficha})"
    );
    Ok(Json(LoginResponse {
        token: token_sessao,
        expira_em,
        dispositivo: novo_dispositivo,
        usuario: UsuarioPublico {
            id: usuario_id,
            login,
            nome,
            admin,
        },
        sessao_id,
        sessao_nome,
        sessao_sistema,
        players_id,
        tem_ficha,
    }))
}

async fn logout(State(estado): State<AppState>, req: Request) -> Result<StatusCode, AppError> {
    if let Some(t) = token_bearer(&req) {
        sqlx::query("DELETE FROM SESSOES_AUTH WHERE token_hash = ?")
            .bind(token::hash(&t))
            .execute(&estado.pool)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn eu(Extension(usuario): Extension<UsuarioAutenticado>) -> Json<UsuarioAutenticado> {
    Json(usuario)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn campos(v: Validador) -> Vec<String> {
        match v.finalizar() {
            Ok(()) => vec![],
            Err(AppError::Validacao(e)) => e.into_iter().map(|c| c.campo).collect(),
            Err(outro) => panic!("{outro:?}"),
        }
    }

    #[test]
    fn login_valido_e_invalidos() {
        for ok in ["ana", "joao.silva", "mestre_01", "a-b"] {
            let mut v = Validador::new();
            validar_login(&mut v, ok);
            assert!(campos(v).is_empty(), "{ok}");
        }
        for ruim in ["ab", "joão", "Ana", ".ana", "ana silva", &"a".repeat(51)] {
            let mut v = Validador::new();
            validar_login(&mut v, ruim);
            assert_eq!(campos(v), vec!["login"], "{ruim}");
        }
    }

    #[test]
    fn senha_igual_ao_login_falha_sem_diferenciar_maiusculas() {
        let mut v = Validador::new();
        validar_senha(&mut v, "Mestre.Joao", "mestre.joao");
        assert_eq!(campos(v), vec!["senha"]);
    }

    #[test]
    fn senha_curta_ou_longa_falha() {
        for (senha, ok) in [
            ("curta", false),
            ("senha-bem-grande", true),
            ("minha-ana-123", true),
            (&*"x".repeat(129), false),
        ] {
            let mut v = Validador::new();
            validar_senha(&mut v, senha, "ana");
            assert_eq!(campos(v).is_empty(), ok, "{senha}");
        }
    }
}
