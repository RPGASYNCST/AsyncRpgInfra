use crate::error::AppError;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, MySqlPool};

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
pub struct FilterSessaoPlayers {
    pub sessao_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessaoPlayer {
    pub sessao_id: u64,
    pub name: String,
    pub player_name: String,
    pub resume: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessaoPlayer {
    pub sessao_id: Option<u64>,
    pub name: Option<String>,
    pub player_name: Option<String>,
    pub resume: Option<String>,
}

pub fn router() -> Router<MySqlPool> {
    Router::new()
        .route("/", get(list_all).post(create))
        .route("/{id}", get(get_by_id).put(update).delete(delete))
}

async fn list_all(
    State(pool): State<MySqlPool>,
    Query(filter): Query<FilterSessaoPlayers>,
) -> Result<Json<Vec<SessaoPlayer>>, AppError> {
    let rows = if let Some(sessao_id) = filter.sessao_id {
        sqlx::query_as::<_, SessaoPlayer>(
            "SELECT id, sessao_id, name, player_name, resume, created_at, updated_at \
             FROM SESSAO_PLAYERS WHERE sessao_id = ? ORDER BY id DESC",
        )
        .bind(sessao_id)
        .fetch_all(&pool)
        .await?
    } else {
        sqlx::query_as::<_, SessaoPlayer>(
            "SELECT id, sessao_id, name, player_name, resume, created_at, updated_at \
             FROM SESSAO_PLAYERS ORDER BY id DESC",
        )
        .fetch_all(&pool)
        .await?
    };

    Ok(Json(rows))
}

async fn get_by_id(
    State(pool): State<MySqlPool>,
    Path(id): Path<u64>,
) -> Result<Json<SessaoPlayer>, AppError> {
    let row = sqlx::query_as::<_, SessaoPlayer>(
        "SELECT id, sessao_id, name, player_name, resume, created_at, updated_at \
         FROM SESSAO_PLAYERS WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("SESSAO_PLAYERS id={id} não encontrado")))?;

    Ok(Json(row))
}

async fn create(
    State(pool): State<MySqlPool>,
    Json(payload): Json<CreateSessaoPlayer>,
) -> Result<(StatusCode, Json<SessaoPlayer>), AppError> {
    let result = sqlx::query(
        "INSERT INTO SESSAO_PLAYERS (sessao_id, name, player_name, resume) VALUES (?, ?, ?, ?)",
    )
    .bind(payload.sessao_id)
    .bind(&payload.name)
    .bind(&payload.player_name)
    .bind(&payload.resume)
    .execute(&pool)
    .await?;

    let created_id = result.last_insert_id();
    let created = get_by_id(State(pool), Path(created_id)).await?.0;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(pool): State<MySqlPool>,
    Path(id): Path<u64>,
    Json(payload): Json<UpdateSessaoPlayer>,
) -> Result<Json<SessaoPlayer>, AppError> {
    let current = get_by_id(State(pool.clone()), Path(id)).await?.0;

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

    get_by_id(State(pool), Path(id)).await
}

async fn delete(
    State(pool): State<MySqlPool>,
    Path(id): Path<u64>,
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
