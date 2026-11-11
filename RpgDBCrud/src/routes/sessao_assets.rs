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
    nome: "SESSAO_ASSETS",
    colunas: "id, sessao_id, glob, description, resume, created_at, updated_at",
    busca: &["glob", "description", "resume"],
    ordenaveis: &[
        ("id", "id"),
        ("sessao_id", "sessao_id"),
        ("created_at", "created_at"),
        ("updated_at", "updated_at"),
    ],
};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SessaoAsset {
    pub id: u64,
    pub sessao_id: u64,
    pub glob: String,
    pub description: Option<String>,
    pub resume: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct FiltroSessaoAssets {
    pub sessao_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessaoAsset {
    pub sessao_id: u64,
    pub glob: String,
    pub description: Option<String>,
    pub resume: Option<String>,
}

impl CreateSessaoAsset {
    fn validar(&self) -> Result<(), AppError> {
        let mut v = Validador::new();
        v.id("sessao_id", self.sessao_id);
        if self.glob.trim().is_empty() {
            v.erro("glob", "é obrigatório e não pode ser vazio");
        }
        v.texto_opcional("glob", Some(&self.glob), TEXT_MAX_BYTES);
        v.texto_opcional("description", self.description.as_deref(), TEXT_MAX_BYTES);
        v.texto_opcional("resume", self.resume.as_deref(), TEXT_MAX_BYTES);
        v.finalizar()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessaoAsset {
    pub sessao_id: Option<u64>,
    pub glob: Option<String>,
    pub description: Option<String>,
    pub resume: Option<String>,
}

impl UpdateSessaoAsset {
    fn validar(&self) -> Result<(), AppError> {
        let mut v = Validador::new();
        v.algum_campo(&[
            self.sessao_id.is_some(),
            self.glob.is_some(),
            self.description.is_some(),
            self.resume.is_some(),
        ]);
        v.id_se_enviado("sessao_id", self.sessao_id);
        if self.glob.as_deref().is_some_and(|g| g.trim().is_empty()) {
            v.erro("glob", "não pode ser vazio");
        }
        v.texto_opcional("glob", self.glob.as_deref(), TEXT_MAX_BYTES);
        v.texto_opcional("description", self.description.as_deref(), TEXT_MAX_BYTES);
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
    Consulta(filtro): Consulta<FiltroSessaoAssets>,
) -> Result<Json<Pagina<SessaoAsset>>, AppError> {
    let mut filtros: Vec<Filtro> = Vec::new();
    if let Some(sessao_id) = filtro.sessao_id {
        filtros.push(("sessao_id", Valor::Id(sessao_id)));
    }
    Ok(Json(
        listagem::listar(&pool, &TABELA, &params, &filtros).await?,
    ))
}

async fn buscar(pool: &MySqlPool, id: u64) -> Result<SessaoAsset, AppError> {
    sqlx::query_as::<_, SessaoAsset>(
        "SELECT id, sessao_id, glob, description, resume, created_at, updated_at \
         FROM SESSAO_ASSETS WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("SESSAO_ASSETS id={id} não encontrado")))
}

async fn get_by_id(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
) -> Result<Json<SessaoAsset>, AppError> {
    Ok(Json(buscar(&pool, id).await?))
}

async fn create(
    State(pool): State<MySqlPool>,
    Corpo(payload): Corpo<CreateSessaoAsset>,
) -> Result<(StatusCode, Json<SessaoAsset>), AppError> {
    payload.validar()?;

    let result = sqlx::query(
        "INSERT INTO SESSAO_ASSETS (sessao_id, glob, description, resume) VALUES (?, ?, ?, ?)",
    )
    .bind(payload.sessao_id)
    .bind(&payload.glob)
    .bind(&payload.description)
    .bind(&payload.resume)
    .execute(&pool)
    .await?;

    let created = buscar(&pool, result.last_insert_id()).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(pool): State<MySqlPool>,
    Caminho(id): Caminho<u64>,
    Corpo(payload): Corpo<UpdateSessaoAsset>,
) -> Result<Json<SessaoAsset>, AppError> {
    payload.validar()?;
    let current = buscar(&pool, id).await?;

    let sessao_id = payload.sessao_id.unwrap_or(current.sessao_id);
    let glob = payload.glob.unwrap_or(current.glob);
    let description = payload.description.or(current.description);
    let resume = payload.resume.or(current.resume);

    sqlx::query(
        "UPDATE SESSAO_ASSETS SET sessao_id = ?, glob = ?, description = ?, resume = ? WHERE id = ?",
    )
    .bind(sessao_id)
    .bind(glob)
    .bind(description)
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
    let result = sqlx::query("DELETE FROM SESSAO_ASSETS WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "SESSAO_ASSETS id={id} não encontrado"
        )));
    }

    Ok(StatusCode::NO_CONTENT)
}
