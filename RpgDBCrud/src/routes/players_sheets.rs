use crate::error::AppError;
use crate::extract::{Caminho, Consulta, Corpo};
use crate::listagem::{self, Filtro, Pagina, ParametrosLista, Tabela, Valor};
use crate::validacao::{Validador, FICHA_MAX_BYTES};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, MySqlPool};

const TABELA: Tabela = Tabela {
    nome: "PLAYERS_SHEETS",
    colunas: "id, sessao_id, players_id, `system`, sheet, created_at, updated_at",
    // A ficha é JSON livre; a busca textual fica restrita ao sistema.
    busca: &["`system`"],
    ordenaveis: &[
        ("id", "id"),
        ("sessao_id", "sessao_id"),
        ("players_id", "players_id"),
        ("system", "`system`"),
        ("created_at", "created_at"),
        ("updated_at", "updated_at"),
    ],
};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlayersSheet {
    pub id: u64,
    pub sessao_id: u64,
    pub players_id: u64,
    pub system: String,
    pub sheet: sqlx::types::Json<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct FiltroPlayersSheets {
    pub sessao_id: Option<u64>,
    pub players_id: Option<u64>,
    pub system: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlayersSheet {
    pub sessao_id: u64,
    pub players_id: u64,
    pub system: String,
    pub sheet: Value,
}

impl CreatePlayersSheet {
    fn validar(&self) -> Result<(), AppError> {
        let mut v = Validador::new();
        v.id("sessao_id", self.sessao_id);
        v.id("players_id", self.players_id);
        v.texto_obrigatorio("system", &self.system, 100);
        v.json_objeto("sheet", &self.sheet, FICHA_MAX_BYTES);
        v.finalizar()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePlayersSheet {
    pub sessao_id: Option<u64>,
    pub players_id: Option<u64>,
    pub system: Option<String>,
    pub sheet: Option<Value>,
}

impl UpdatePlayersSheet {
    fn validar(&self) -> Result<(), AppError> {
        let mut v = Validador::new();
        v.algum_campo(&[
            self.sessao_id.is_some(),
            self.players_id.is_some(),
            self.system.is_some(),
            self.sheet.is_some(),
        ]);
        v.id_se_enviado("sessao_id", self.sessao_id);
        v.id_se_enviado("players_id", self.players_id);
        v.texto_obrigatorio_se_enviado("system", self.system.as_deref(), 100);
        if let Some(sheet) = &self.sheet {
            v.json_objeto("sheet", sheet, FICHA_MAX_BYTES);
        }
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
    Consulta(filtro): Consulta<FiltroPlayersSheets>,
) -> Result<Json<Pagina<PlayersSheet>>, AppError> {
    let mut filtros: Vec<Filtro> = Vec::new();
    if let Some(sessao_id) = filtro.sessao_id {
        filtros.push(("sessao_id", Valor::Id(sessao_id)));
    }
    if let Some(players_id) = filtro.players_id {
        filtros.push(("players_id", Valor::Id(players_id)));
    }
    if let Some(system) = filtro.system {
        filtros.push(("`system`", Valor::Texto(system)));
    }
    Ok(Json(
        listagem::listar(&pool, &TABELA, &params, &filtros).await?,
    ))
}

async fn buscar(pool: &MySqlPool, id: u64) -> Result<PlayersSheet, AppError> {
    sqlx::query_as::<_, PlayersSheet>(
        "SELECT id, sessao_id, players_id, `system`, sheet, created_at, updated_at \
         FROM PLAYERS_SHEETS WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("PLAYERS_SHEETS id={id} não encontrada")))
}

async fn get_by_id(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
) -> Result<Json<PlayersSheet>, AppError> {
    Ok(Json(buscar(&pool, id).await?))
}

async fn create(
    State(pool): State<MySqlPool>,
    Corpo(payload): Corpo<CreatePlayersSheet>,
) -> Result<(StatusCode, Json<PlayersSheet>), AppError> {
    payload.validar()?;

    let result = sqlx::query(
        "INSERT INTO PLAYERS_SHEETS (sessao_id, players_id, `system`, sheet) VALUES (?, ?, ?, ?)",
    )
    .bind(payload.sessao_id)
    .bind(payload.players_id)
    .bind(&payload.system)
    .bind(sqlx::types::Json(&payload.sheet))
    .execute(&pool)
    .await?;

    let created = buscar(&pool, result.last_insert_id()).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
    Corpo(payload): Corpo<UpdatePlayersSheet>,
) -> Result<Json<PlayersSheet>, AppError> {
    payload.validar()?;
    let current = buscar(&pool, id).await?;

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

    Ok(Json(buscar(&pool, id).await?))
}

async fn delete(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
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
