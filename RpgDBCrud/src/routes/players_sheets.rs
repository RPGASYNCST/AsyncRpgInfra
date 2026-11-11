use crate::error::AppError;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, MySqlPool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlayersSheet {
    pub id: u64,
    pub sessao_id: u64,
    pub players_id: u64,
    pub system: String,
    pub sheet: sqlx::types::Json<Value>,
    pub created_at: DateTime<Utc>,
    #[serde(alias = "update_at")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct FilterPlayersSheets {
    pub sessao_id: Option<u64>,
    pub players_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlayersSheet {
    pub sessao_id: u64,
    pub players_id: u64,
    pub system: String,
    pub sheet: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePlayersSheet {
    pub sessao_id: Option<u64>,
    pub players_id: Option<u64>,
    pub system: Option<String>,
    pub sheet: Option<Value>,
}

pub fn router() -> Router<MySqlPool> {
    Router::new()
        .route("/", get(list_all).post(create))
        .route("/{id}", get(get_by_id).put(update).delete(delete))
}

async fn list_all(
    State(pool): State<MySqlPool>,
    Query(filter): Query<FilterPlayersSheets>,
) -> Result<Json<Vec<PlayersSheet>>, AppError> {
    let rows = match (filter.sessao_id, filter.players_id) {
        (Some(sessao_id), Some(players_id)) => {
            sqlx::query_as::<_, PlayersSheet>(
                "SELECT id, sessao_id, players_id, `system`, sheet, created_at, updated_at \
                 FROM PLAYERS_SHEETS WHERE sessao_id = ? AND players_id = ? ORDER BY id DESC",
            )
            .bind(sessao_id)
            .bind(players_id)
            .fetch_all(&pool)
            .await?
        }
        (Some(sessao_id), None) => {
            sqlx::query_as::<_, PlayersSheet>(
                "SELECT id, sessao_id, players_id, `system`, sheet, created_at, updated_at \
                 FROM PLAYERS_SHEETS WHERE sessao_id = ? ORDER BY id DESC",
            )
            .bind(sessao_id)
            .fetch_all(&pool)
            .await?
        }
        (None, Some(players_id)) => {
            sqlx::query_as::<_, PlayersSheet>(
                "SELECT id, sessao_id, players_id, `system`, sheet, created_at, updated_at \
                 FROM PLAYERS_SHEETS WHERE players_id = ? ORDER BY id DESC",
            )
            .bind(players_id)
            .fetch_all(&pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, PlayersSheet>(
                "SELECT id, sessao_id, players_id, `system`, sheet, created_at, updated_at \
                 FROM PLAYERS_SHEETS ORDER BY id DESC",
            )
            .fetch_all(&pool)
            .await?
        }
    };

    Ok(Json(rows))
}

async fn get_by_id(
    State(pool): State<MySqlPool>,
    Path(id): Path<u64>,
) -> Result<Json<PlayersSheet>, AppError> {
    let row = sqlx::query_as::<_, PlayersSheet>(
        "SELECT id, sessao_id, players_id, `system`, sheet, created_at, updated_at \
         FROM PLAYERS_SHEETS WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("PLAYERS_SHEETS id={id} não encontrada")))?;

    Ok(Json(row))
}

async fn create(
    State(pool): State<MySqlPool>,
    Json(payload): Json<CreatePlayersSheet>,
) -> Result<(StatusCode, Json<PlayersSheet>), AppError> {
    let result = sqlx::query(
        "INSERT INTO PLAYERS_SHEETS (sessao_id, players_id, `system`, sheet) VALUES (?, ?, ?, ?)",
    )
    .bind(payload.sessao_id)
    .bind(payload.players_id)
    .bind(&payload.system)
    .bind(sqlx::types::Json(&payload.sheet))
    .execute(&pool)
    .await?;

    let created_id = result.last_insert_id();
    let created = get_by_id(State(pool), Path(created_id)).await?.0;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(pool): State<MySqlPool>,
    Path(id): Path<u64>,
    Json(payload): Json<UpdatePlayersSheet>,
) -> Result<Json<PlayersSheet>, AppError> {
    let current = get_by_id(State(pool.clone()), Path(id)).await?.0;

    let sessao_id = payload.sessao_id.unwrap_or(current.sessao_id);
    let players_id = payload.players_id.unwrap_or(current.players_id);
    let system = payload.system.unwrap_or(current.system);
    let sheet = payload.sheet.unwrap_or(current.sheet.0);

    sqlx::query(
        "UPDATE PLAYERS_SHEETS SET sessao_id = ?, players_id = ?, `system` = ?, sheet = ? WHERE id = ?",
    )
    .bind(sessao_id)
    .bind(players_id)
    .bind(system)
    .bind(sqlx::types::Json(sheet))
    .bind(id)
    .execute(&pool)
    .await?;

    get_by_id(State(pool), Path(id)).await
}

async fn delete(
    State(pool): State<MySqlPool>,
    Path(id): Path<u64>,
) -> Result<StatusCode, AppError> {
    let result = sqlx::query("DELETE FROM PLAYERS_SHEETS WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "PLAYERS_SHEETS id={id} não encontrada"
        )));
    }

    Ok(StatusCode::NO_CONTENT)
}