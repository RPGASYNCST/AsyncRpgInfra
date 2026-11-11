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
    nome: "SESSAO_RPG",
    colunas: "id, name, description, `system`, created_at, updated_at",
    busca: &["name", "description"],
    ordenaveis: &[
        ("id", "id"),
        ("name", "name"),
        ("system", "`system`"),
        ("created_at", "created_at"),
        ("updated_at", "updated_at"),
    ],
};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SessaoRpg {
    pub id: u64,
    pub name: String,
    pub description: Option<String>,
    pub system: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Filtros exatos desta tabela (além dos parâmetros comuns de listagem).
#[derive(Debug, Deserialize)]
pub struct FiltroSessaoRpg {
    pub system: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessaoRpg {
    pub name: String,
    pub description: Option<String>,
    pub system: String,
}

impl CreateSessaoRpg {
    fn validar(&self) -> Result<(), AppError> {
        let mut v = Validador::new();
        v.texto_obrigatorio("name", &self.name, 255);
        v.texto_opcional("description", self.description.as_deref(), TEXT_MAX_BYTES);
        v.texto_obrigatorio("system", &self.system, 100);
        v.finalizar()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessaoRpg {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system: Option<String>,
}

impl UpdateSessaoRpg {
    fn validar(&self) -> Result<(), AppError> {
        let mut v = Validador::new();
        v.algum_campo(&[
            self.name.is_some(),
            self.description.is_some(),
            self.system.is_some(),
        ]);
        v.texto_obrigatorio_se_enviado("name", self.name.as_deref(), 255);
        v.texto_opcional("description", self.description.as_deref(), TEXT_MAX_BYTES);
        v.texto_obrigatorio_se_enviado("system", self.system.as_deref(), 100);
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
    Consulta(filtro): Consulta<FiltroSessaoRpg>,
) -> Result<Json<Pagina<SessaoRpg>>, AppError> {
    let mut filtros: Vec<Filtro> = Vec::new();
    if let Some(system) = filtro.system {
        filtros.push(("`system`", Valor::Texto(system)));
    }
    Ok(Json(
        listagem::listar(&pool, &TABELA, &params, &filtros).await?,
    ))
}

async fn buscar(pool: &MySqlPool, id: u64) -> Result<SessaoRpg, AppError> {
    sqlx::query_as::<_, SessaoRpg>(
        "SELECT id, name, description, `system`, created_at, updated_at FROM SESSAO_RPG WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("SESSAO_RPG id={id} não encontrada")))
}

async fn get_by_id(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
) -> Result<Json<SessaoRpg>, AppError> {
    Ok(Json(buscar(&pool, id).await?))
}

async fn create(
    State(pool): State<MySqlPool>,
    Corpo(payload): Corpo<CreateSessaoRpg>,
) -> Result<(StatusCode, Json<SessaoRpg>), AppError> {
    payload.validar()?;

    let result =
        sqlx::query("INSERT INTO SESSAO_RPG (name, description, `system`) VALUES (?, ?, ?)")
            .bind(&payload.name)
            .bind(&payload.description)
            .bind(&payload.system)
            .execute(&pool)
            .await?;

    let created = buscar(&pool, result.last_insert_id()).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
    Corpo(payload): Corpo<UpdateSessaoRpg>,
) -> Result<Json<SessaoRpg>, AppError> {
    payload.validar()?;
    let current = buscar(&pool, id).await?;

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

    Ok(Json(buscar(&pool, id).await?))
}

async fn delete(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
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
