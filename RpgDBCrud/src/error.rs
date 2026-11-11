use crate::validacao::ErroCampo;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use sqlx::error::ErrorKind;

/// Todas as respostas de erro saem como JSON:
/// `{"error": "mensagem"}` e, em falhas de validação, também `"campos": [{"campo", "mensagem"}]`.
#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    /// Campos inválidos (422).
    Validacao(Vec<ErroCampo>),
    /// Requisição malformada rejeitada antes do handler (JSON, query ou caminho inválidos).
    Entrada(StatusCode, String),
    /// Falta token de sessão ou ele é inválido/expirado (401).
    NaoAutenticado,
    /// Autenticado, mas sem permissão (403).
    Proibido(String),
    /// Login ou senha errados (401). `aguarde_segundos` = espera antes da próxima tentativa.
    LoginRecusado {
        aguarde_segundos: i64,
    },
    /// Ainda dentro da espera da penalidade (429).
    MuitasTentativas {
        aguarde_segundos: i64,
    },
    /// Falha interna fora do banco (ex.: gerador aleatório); detalhe só no log (500).
    Interno(String),
    Database(sqlx::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        if let sqlx::Error::RowNotFound = err {
            return Self::NotFound("Registro não encontrado".to_string());
        }

        // Erros de integridade gerados pelo próprio MariaDB viram 4xx com mensagem genérica.
        // Nenhuma regra de negócio é aplicada aqui: só traduzimos o erro do banco para HTTP.
        if let Some(db_err) = err.as_database_error() {
            match db_err.kind() {
                ErrorKind::ForeignKeyViolation => {
                    return Self::Conflict(
                        "Registro relacionado inexistente ou ainda referenciado por outro registro."
                            .to_string(),
                    );
                }
                ErrorKind::UniqueViolation => {
                    return Self::Conflict("Registro duplicado.".to_string());
                }
                ErrorKind::NotNullViolation | ErrorKind::CheckViolation => {
                    return Self::BadRequest("Dados inválidos para a tabela.".to_string());
                }
                _ => {}
            }
            // SQLSTATE classe 22 = dado inválido (ex.: texto maior que a coluna, JSON inválido).
            if db_err.code().is_some_and(|code| code.starts_with("22")) {
                return Self::BadRequest(
                    "Dados inválidos para a tabela (tamanho ou formato).".to_string(),
                );
            }
        }

        Self::Database(err)
    }
}

impl From<JsonRejection> for AppError {
    fn from(rej: JsonRejection) -> Self {
        Self::Entrada(
            rej.status(),
            format!("Corpo JSON inválido: {}", rej.body_text()),
        )
    }
}

impl From<QueryRejection> for AppError {
    fn from(rej: QueryRejection) -> Self {
        Self::Entrada(
            StatusCode::BAD_REQUEST,
            format!("Parâmetros de consulta inválidos: {}", rej.body_text()),
        )
    }
}

impl From<PathRejection> for AppError {
    fn from(rej: PathRejection) -> Self {
        Self::Entrada(
            StatusCode::BAD_REQUEST,
            format!("Parâmetro de caminho inválido: {}", rej.body_text()),
        )
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, json!({ "error": msg })),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, json!({ "error": msg })),
            Self::Conflict(msg) => (StatusCode::CONFLICT, json!({ "error": msg })),
            Self::Validacao(campos) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "Dados inválidos.", "campos": campos }),
            ),
            Self::Entrada(status, msg) => (status, json!({ "error": msg })),
            Self::NaoAutenticado => {
                return (
                    StatusCode::UNAUTHORIZED,
                    [(header::WWW_AUTHENTICATE, "Bearer".to_string())],
                    Json(json!({ "error": "Faça login para usar a API." })),
                )
                    .into_response();
            }
            Self::Proibido(msg) => (StatusCode::FORBIDDEN, json!({ "error": msg })),
            Self::LoginRecusado { aguarde_segundos } => {
                return resposta_com_espera(
                    StatusCode::UNAUTHORIZED,
                    "Usuário ou senha inválidos.",
                    aguarde_segundos,
                );
            }
            Self::MuitasTentativas { aguarde_segundos } => {
                return resposta_com_espera(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Muitas tentativas. Aguarde antes de tentar de novo.",
                    aguarde_segundos,
                );
            }
            Self::Interno(detalhe) => {
                tracing::error!("Erro interno: {detalhe}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({ "error": "Erro interno." }),
                )
            }
            Self::Database(err) => {
                // O detalhe fica só no log do servidor; o cliente não recebe estrutura interna do banco.
                tracing::error!("Erro no banco de dados MariaDB: {err}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({ "error": "Erro interno ao acessar o banco de dados." }),
                )
            }
        };

        (status, Json(body)).into_response()
    }
}

/// Resposta com `aguarde_segundos` no corpo e `Retry-After` no cabeçalho (quando > 0).
fn resposta_com_espera(status: StatusCode, mensagem: &str, aguarde_segundos: i64) -> Response {
    let corpo = Json(json!({ "error": mensagem, "aguarde_segundos": aguarde_segundos }));
    if aguarde_segundos > 0 {
        (
            status,
            [(header::RETRY_AFTER, aguarde_segundos.to_string())],
            corpo,
        )
            .into_response()
    } else {
        (status, corpo).into_response()
    }
}
