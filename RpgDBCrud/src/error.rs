use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
#[allow(dead_code)]
pub enum AppError {
    NotFound(String),
    BadRequest(String),
    Database(sqlx::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => Self::NotFound("Registro não encontrado".to_string()),
            other => Self::Database(other),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            Self::Database(err) => {
                tracing::error!("Erro no banco de dados MariaDB: {err}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Erro no banco de dados: {err}"),
                )
            }
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

