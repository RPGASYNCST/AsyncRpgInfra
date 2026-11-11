//! Extratores que trocam as respostas de erro em texto puro do Axum por JSON no padrão da
//! API (`{"error": "..."}`). Use `Corpo`, `Consulta` e `Caminho` nos handlers no lugar de
//! `Json`, `Query` e `Path`.

use crate::error::AppError;
use axum::extract::{FromRequest, FromRequestParts};

/// Corpo JSON da requisição (substitui `axum::Json` na entrada).
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub struct Corpo<T>(pub T);

/// Parâmetros da query string (substitui `axum::extract::Query`).
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(AppError))]
pub struct Consulta<T>(pub T);

/// Parâmetros do caminho, ex.: `{id}` (substitui `axum::extract::Path`).
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(AppError))]
pub struct Caminho<T>(pub T);
