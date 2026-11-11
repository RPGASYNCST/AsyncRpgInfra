use crate::error::AppError;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, MySqlPool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SessaoRpg {
    pub id: u64,
    pub name: String,
    pub description: Option<String>,
    pub system: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessaoRpg {
    pub name: String,
    pub description: Option<String>,
    pub system: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessaoRpg {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system: Option<String>,
}

pub fn router() -> Router<MySqlPool> {
    Router::new()
        .route("/", get(list_all).post(create))
        .route("/{id}", get(get_by_id).put(update).delete(delete))
}

async fn list_all(State(pool): State<MySqlPool>) -> Result<Json<Vec<SessaoRpg>>, AppError> {
    let rows = sqlx::query_as::<_, SessaoRpg>(
        "SELECT id, name, description, `system`, created_at, updated_at FROM SESSAO_RPG ORDER BY id DESC",
    )
    .fetch_all(&pool)
    .await?;

    Ok(Json(rows))
}

async fn get_by_id(
    State(pool): State<MySqlPool>,
    Path(id): Path<u64>,
) -> Result<Json<SessaoRpg>, AppError> {
    let row = sqlx::query_as::<_, SessaoRpg>(
        "SELECT id, name, description, `system`, created_at, updated_at FROM SESSAO_RPG WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("SESSAO_RPG id={id} não encontrada")))?;

    Ok(Json(row))
}

async fn create(
    State(pool): State<MySqlPool>,
    Json(payload): Json<CreateSessaoRpg>,
) -> Result<(StatusCode, Json<SessaoRpg>), AppError> {
    let result =
        sqlx::query("INSERT INTO SESSAO_RPG (name, description, `system`) VALUES (?, ?, ?)")
            .bind(&payload.name)
            .bind(&payload.description)
            .bind(&payload.system)
            .execute(&pool)
            .await?;

    let created_id = result.last_insert_id();
    let created = get_by_id(State(pool), Path(created_id)).await?.0;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(pool): State<MySqlPool>,
    Path(id): Path<u64>,
    Json(payload): Json<UpdateSessaoRpg>,
) -> Result<Json<SessaoRpg>, AppError> {
    let current = get_by_id(State(pool.clone()), Path(id)).await?.0;

    let name = payload.name.unwrap_or(current.name);
    let description = payload.description.or(current.description);
    let system = payload.system.unwrap_or(current.system);

    sqlx::query("UPDATE SESSAO_RPG SET name = ?, description = ?, `system` = ? WHERE id = ?")
        .bind(name)
        .bind(description)
        .bind(system)
        .bind(id)
        .execute(&pool)
        .await?;

    get_by_id(State(pool), Path(id)).await
}

async fn delete(
    State(pool): State<MySqlPool>,
    Path(id): Path<u64>,
) -> Result<StatusCode, AppError> {
    let result = sqlx::query("DELETE FROM SESSAO_RPG WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "SESSAO_RPG id={id} não encontrada"
        )));
    }

    Ok(StatusCode::NO_CONTENT)
}
