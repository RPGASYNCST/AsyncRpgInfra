use crate::auth::token;
use crate::error::AppError;
use crate::estado::AppState;
use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use chrono::Utc;
use serde::Serialize;
use sqlx::MySqlPool;

/// Usuário dono do token da requisição; fica disponível nos handlers via
/// `Extension<UsuarioAutenticado>`.
#[derive(Debug, Clone, Serialize)]
pub struct UsuarioAutenticado {
    pub id: u64,
    pub login: String,
    pub nome: String,
    pub admin: bool,
}

/// Token enviado em `Authorization: Bearer <token>`.
pub fn token_bearer(req: &Request) -> Option<String> {
    let valor = req.headers().get(AUTHORIZATION)?.to_str().ok()?;
    let token = valor.strip_prefix("Bearer ")?.trim();
    token::formato_valido(token).then(|| token.to_string())
}

pub async fn usuario_do_token(
    pool: &MySqlPool,
    token: &str,
) -> Result<Option<UsuarioAutenticado>, AppError> {
    let linha: Option<(u64, String, String, bool)> = sqlx::query_as(
        "SELECT u.id, u.login, u.nome, u.admin FROM SESSOES_AUTH s \
         JOIN USUARIOS u ON u.id = s.usuario_id \
         WHERE s.token_hash = ? AND s.expira_em > ? AND u.ativo = TRUE",
    )
    .bind(token::hash(token))
    .bind(Utc::now())
    .fetch_optional(pool)
    .await?;
    Ok(linha.map(|(id, login, nome, admin)| UsuarioAutenticado {
        id,
        login,
        nome,
        admin,
    }))
}

/// Exige um token de sessão válido (401 caso contrário).
pub async fn exigir_login(
    State(estado): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = token_bearer(&req).ok_or(AppError::NaoAutenticado)?;
    let usuario = usuario_do_token(&estado.pool, &token)
        .await?
        .ok_or(AppError::NaoAutenticado)?;
    req.extensions_mut().insert(usuario);
    Ok(next.run(req).await)
}

/// Exige perfil de admin (403). Deve rodar depois de [`exigir_login`].
pub async fn exigir_admin(req: Request, next: Next) -> Result<Response, AppError> {
    match req.extensions().get::<UsuarioAutenticado>() {
        Some(u) if u.admin => Ok(next.run(req).await),
        Some(_) => Err(AppError::Proibido(
            "Apenas administradores podem usar esta rota.".to_string(),
        )),
        None => Err(AppError::NaoAutenticado),
    }
}
