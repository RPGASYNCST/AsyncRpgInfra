//! Lógica de verificação de e-mail, gravação atômica de usuário no MariaDB e página HTML NeXTSTEP 1996.

use crate::seguranca::{
    self, chave_conta_verif, chave_ip, ip_cliente, segundos_ate, SOLICITACOES_LIVRES_POR_IP,
};
use crate::VerifierState;
use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, MySqlPool};
use std::net::SocketAddr;

const HTML_TEMPLATE: &str = include_str!("../static/verify.html");
const LOGO_PNG: &[u8] = include_bytes!("../static/logo.png");
const FONT_SILKSCREEN_BOLD: &[u8] = include_bytes!("../static/fonts/Silkscreen-Bold.ttf");
const FONT_SILKSCREEN_REGULAR: &[u8] = include_bytes!("../static/fonts/Silkscreen-Regular.ttf");
const FONT_VT323_REGULAR: &[u8] = include_bytes!("../static/fonts/VT323-Regular.ttf");

const VALIDADE_VERIFICACAO_HORAS: i64 = 24;
const LOGIN_MIN: usize = 3;
const LOGIN_MAX: usize = 50;
const SENHA_MIN: usize = 6;
const SENHA_MAX: usize = 128;

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(VerifierError))]
pub struct Corpo<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(VerifierError))]
pub struct Consulta<T>(pub T);

#[derive(Debug)]
pub enum VerifierError {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    TooManyRequests { aguarde_segundos: i64 },
    Internal(String),
}

impl From<sqlx::Error> for VerifierError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!("Erro de banco no RpgEmailVerifier: {err}");
        Self::Internal("Erro ao acessar o banco de dados.".to_string())
    }
}

impl From<JsonRejection> for VerifierError {
    fn from(rej: JsonRejection) -> Self {
        Self::BadRequest(format!("Corpo JSON inválido: {}", rej.body_text()))
    }
}

impl From<QueryRejection> for VerifierError {
    fn from(rej: QueryRejection) -> Self {
        Self::BadRequest(format!(
            "Parâmetros de consulta inválidos: {}",
            rej.body_text()
        ))
    }
}

impl IntoResponse for VerifierError {
    fn into_response(self) -> Response {
        match self {
            Self::TooManyRequests { aguarde_segundos } => {
                let corpo = Json(json!({
                    "error": "Muitas tentativas. Aguarde antes de tentar novamente.",
                    "aguarde_segundos": aguarde_segundos
                }));
                if aguarde_segundos > 0 {
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        [(header::RETRY_AFTER, aguarde_segundos.to_string())],
                        corpo,
                    )
                        .into_response()
                } else {
                    (StatusCode::TOO_MANY_REQUESTS, corpo).into_response()
                }
            }
            outro => {
                let (status, msg) = match outro {
                    Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
                    Self::NotFound(m) => (StatusCode::NOT_FOUND, m),
                    Self::Conflict(m) => (StatusCode::CONFLICT, m),
                    Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
                    Self::TooManyRequests { .. } => unreachable!(),
                };
                (status, Json(json!({ "error": msg }))).into_response()
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SolicitarVerificacaoRequest {
    pub login: String,
    pub nome: Option<String>,
    pub email: String,
    pub senha: String,
    pub papel: Option<String>,
    pub sessao_id: Option<u64>,
    pub sessao_nome: Option<String>,
    pub sessao_sistema: Option<String>,
    pub sessao_descricao: Option<String>,
    pub codigo_convite: Option<String>,
    pub servidor: Option<String>,
    pub codigo: Option<String>,
    pub token: Option<String>,
    pub enviar_email: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct SolicitarVerificacaoResponse {
    pub token: String,
    pub codigo: String,
    pub verify_url: String,
    pub login: String,
    pub nome: String,
    pub email: String,
    pub papel: String,
    pub sessao_nome: String,
    pub servidor: String,
    pub email_enviado: bool,
    pub expira_em: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmarVerificacaoRequest {
    pub token: Option<String>,
    pub codigo: Option<String>,
    pub login: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConfirmacaoResponse {
    pub confirmado: bool,
    pub usuario_id: u64,
    pub token: String,
    pub login: String,
    pub handle: String,
    pub email: String,
    pub papel: String,
    pub sessao_nome: String,
    pub servidor: String,
    pub status_conexao: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyQuery {
    pub token: Option<String>,
    pub code: Option<String>,
    pub login: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct VerificacaoRow {
    id: u64,
    token: String,
    login: String,
    nome: String,
    email: String,
    senha_hash: String,
    papel: String,
    sessao_id: Option<u64>,
    sessao_nome: String,
    sessao_sistema: String,
    sessao_descricao: Option<String>,
    codigo_convite: Option<String>,
    servidor: String,
    confirmado: bool,
    usuario_id: Option<u64>,
    expira_em: DateTime<Utc>,
}

pub fn router() -> Router<VerifierState> {
    Router::new()
        .route("/", get(pagina_verificacao_html))
        .route("/solicitar", post(solicitar_verificacao))
        .route("/confirmar", post(confirmar_verificacao_json))
        .route("/status", get(status_verificacao_json))
        .route("/assets/logo.png", get(servir_logo))
        .route(
            "/assets/fonts/Silkscreen-Bold.ttf",
            get(servir_font_silkscreen_bold),
        )
        .route(
            "/assets/fonts/Silkscreen-Regular.ttf",
            get(servir_font_silkscreen_regular),
        )
        .route(
            "/assets/fonts/VT323-Regular.ttf",
            get(servir_font_vt323_regular),
        )
}

/// Garante que as tabelas e colunas necessárias existam no MariaDB ao iniciar.
pub async fn garantir_schema_verificacao(pool: &MySqlPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS USUARIOS (
            id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
            login VARCHAR(50) NOT NULL,
            nome VARCHAR(255) NOT NULL,
            email VARCHAR(255) NULL,
            senha_hash VARCHAR(255) NOT NULL,
            admin BOOLEAN NOT NULL DEFAULT FALSE,
            ativo BOOLEAN NOT NULL DEFAULT TRUE,
            email_verificado BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
            PRIMARY KEY (id),
            UNIQUE KEY uk_usuarios_login (login)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "ALTER TABLE USUARIOS
            ADD COLUMN IF NOT EXISTS email VARCHAR(255) NULL AFTER nome,
            ADD COLUMN IF NOT EXISTS email_verificado BOOLEAN NOT NULL DEFAULT FALSE AFTER ativo",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS PENALIDADES (
            chave VARCHAR(191) NOT NULL,
            falhas INT UNSIGNED NOT NULL DEFAULT 0,
            bloqueado_ate DATETIME(3) NULL,
            ultima_falha_em DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
            PRIMARY KEY (chave),
            INDEX idx_penalidades_ultima_falha (ultima_falha_em)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS VERIFICACOES_CADASTRO (
            id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
            token VARCHAR(64) NOT NULL,
            codigo VARCHAR(16) NOT NULL,
            login VARCHAR(50) NOT NULL,
            nome VARCHAR(255) NOT NULL,
            email VARCHAR(255) NOT NULL,
            senha_hash VARCHAR(255) NOT NULL,
            papel VARCHAR(100) NOT NULL DEFAULT 'Mestre (Host)',
            sessao_id BIGINT UNSIGNED NULL,
            sessao_nome VARCHAR(255) NOT NULL DEFAULT 'Crônicas de Kharos',
            sessao_sistema VARCHAR(100) NOT NULL DEFAULT 'Old Dragon / D&D 5e',
            sessao_descricao TEXT NULL,
            codigo_convite VARCHAR(100) NULL,
            servidor VARCHAR(255) NOT NULL DEFAULT '127.0.0.1:8080',
            confirmado BOOLEAN NOT NULL DEFAULT FALSE,
            usuario_id BIGINT UNSIGNED NULL,
            expira_em DATETIME(3) NOT NULL,
            confirmado_em DATETIME(3) NULL,
            created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
            PRIMARY KEY (id),
            UNIQUE KEY uk_verificacoes_token (token),
            INDEX idx_verificacoes_codigo_login (codigo, login),
            INDEX idx_verificacoes_email (email),
            INDEX idx_verificacoes_expira (expira_em)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "ALTER TABLE VERIFICACOES_CADASTRO
            ADD COLUMN IF NOT EXISTS sessao_sistema VARCHAR(100) NOT NULL DEFAULT 'Old Dragon / D&D 5e' AFTER sessao_nome,
            ADD COLUMN IF NOT EXISTS sessao_descricao TEXT NULL AFTER sessao_sistema",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Remove periodicamente solicitações de verificação expiradas e não confirmadas.
pub async fn limpar_verificacoes_expiradas(pool: &MySqlPool) -> Result<u64, sqlx::Error> {
    let removidas =
        sqlx::query("DELETE FROM VERIFICACOES_CADASTRO WHERE confirmado = FALSE AND expira_em < ?")
            .bind(Utc::now())
            .execute(pool)
            .await?
            .rows_affected();

    if removidas > 0 {
        tracing::info!(
            "Limpeza RpgEmailVerifier: {removidas} verificação(ões) expirada(s) removida(s)."
        );
    }
    Ok(removidas)
}

async fn servir_logo() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        LOGO_PNG,
    )
        .into_response()
}

async fn servir_font_silkscreen_bold() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "font/ttf"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FONT_SILKSCREEN_BOLD,
    )
        .into_response()
}

async fn servir_font_silkscreen_regular() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "font/ttf"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FONT_SILKSCREEN_REGULAR,
    )
        .into_response()
}

async fn servir_font_vt323_regular() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "font/ttf"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FONT_VT323_REGULAR,
    )
        .into_response()
}

/// Gera hash Argon2id com os mesmos parâmetros do `RpgDBCrud` (19 MiB, t=2, p=1).
async fn gerar_hash_senha(senha: String) -> Result<String, VerifierError> {
    tokio::task::spawn_blocking(move || {
        let mut salt_bytes = [0u8; 16];
        getrandom::fill(&mut salt_bytes)
            .map_err(|e| VerifierError::Internal(format!("Erro no gerador aleatório: {e}")))?;
        let salt = SaltString::encode_b64(&salt_bytes)
            .map_err(|e| VerifierError::Internal(format!("Erro ao gerar salt: {e}")))?;
        let params = Params::new(19 * 1024, 2, 1, None)
            .map_err(|e| VerifierError::Internal(format!("Parâmetros Argon2: {e}")))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        argon2
            .hash_password(senha.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| VerifierError::Internal(format!("Erro ao calcular hash Argon2: {e}")))
    })
    .await
    .map_err(|e| VerifierError::Internal(format!("Task Argon2 interrompida: {e}")))?
}

/// Gera token no padrão `ACT-98-XXXX-YYY` (ex.: `ACT-98-7741-KHR`).
fn gerar_token_ativacao(sufixo_convite: Option<&str>) -> String {
    let mut bytes = [0u8; 4];
    let _ = getrandom::fill(&mut bytes);
    let numero: u32 = 1000 + (u32::from_le_bytes(bytes) % 9000);

    let limpo: String = sufixo_convite
        .unwrap_or("KHR")
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .collect();

    let consoantes: String = limpo
        .chars()
        .filter(|c| !matches!(c, 'A' | 'E' | 'I' | 'O' | 'U'))
        .take(3)
        .collect();

    let sufixo_final = if consoantes.len() == 3 {
        consoantes
    } else if limpo.len() >= 3 {
        limpo.chars().take(3).collect()
    } else {
        "KHR".to_string()
    };

    format!("ACT-98-{numero}-{sufixo_final}")
}

/// Gera código numérico de 6 dígitos (ex.: `482910`).
fn gerar_codigo_6_digitos() -> String {
    let mut bytes = [0u8; 4];
    let _ = getrandom::fill(&mut bytes);
    let codigo: u32 = 100_000 + (u32::from_le_bytes(bytes) % 900_000);
    codigo.to_string()
}

fn validar_token_customizado(token: &str) -> Result<(), VerifierError> {
    if token.len() < 8
        || token.len() > 64
        || !token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(VerifierError::BadRequest(
            "Formato de token inválido (use 8 a 64 caracteres alfanuméricos ou '-').".to_string(),
        ));
    }
    Ok(())
}

fn validar_codigo_customizado(codigo: &str) -> Result<(), VerifierError> {
    if codigo.len() != 6 || !codigo.chars().all(|c| c.is_ascii_digit()) {
        return Err(VerifierError::BadRequest(
            "O código de confirmação deve conter exatamente 6 dígitos numéricos.".to_string(),
        ));
    }
    Ok(())
}

fn validar_dados_cadastro(
    login: &str,
    nome: &str,
    email: &str,
    senha: &str,
    papel: &str,
    servidor: &str,
) -> Result<(), VerifierError> {
    let tam_login = login.chars().count();
    if !(LOGIN_MIN..=LOGIN_MAX).contains(&tam_login) {
        return Err(VerifierError::BadRequest(format!(
            "O login deve ter entre {LOGIN_MIN} e {LOGIN_MAX} caracteres."
        )));
    }
    if !login
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        || !login.starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        return Err(VerifierError::BadRequest(
            "Use apenas letras, números, '.', '_' e '-' no login, começando com letra ou número."
                .to_string(),
        ));
    }

    if nome.is_empty() || nome.chars().count() > 255 || nome.chars().any(char::is_control) {
        return Err(VerifierError::BadRequest(
            "O nome/handle é inválido ou excede 255 caracteres.".to_string(),
        ));
    }

    let partes_email: Vec<&str> = email.split('@').collect();
    if email.len() > 255
        || email.chars().any(|c| c.is_control() || c.is_whitespace())
        || partes_email.len() != 2
        || partes_email[0].is_empty()
        || !partes_email[1].contains('.')
        || partes_email[1].starts_with('.')
        || partes_email[1].ends_with('.')
    {
        return Err(VerifierError::BadRequest(
            "Informe um endereço de e-mail válido.".to_string(),
        ));
    }

    let tam_senha = senha.chars().count();
    if !(SENHA_MIN..=SENHA_MAX).contains(&tam_senha) {
        return Err(VerifierError::BadRequest(format!(
            "A chave de acesso deve ter entre {SENHA_MIN} e {SENHA_MAX} caracteres."
        )));
    }
    if senha.to_lowercase() == login {
        return Err(VerifierError::BadRequest(
            "A chave de acesso não pode ser igual ao login.".to_string(),
        ));
    }

    if papel.chars().count() > 100 || papel.chars().any(char::is_control) {
        return Err(VerifierError::BadRequest(
            "O papel na mesa é inválido ou excede 100 caracteres.".to_string(),
        ));
    }

    if servidor.chars().count() > 255 || servidor.chars().any(char::is_control) {
        return Err(VerifierError::BadRequest(
            "O endereço do servidor é inválido ou excede 255 caracteres.".to_string(),
        ));
    }

    Ok(())
}

async fn resolver_sessao(
    pool: &MySqlPool,
    sessao_id: Option<u64>,
    sessao_nome: Option<&str>,
    codigo_convite: Option<&str>,
) -> Result<(Option<u64>, String), VerifierError> {
    if let Some(id) = sessao_id {
        if let Ok(Some((id_enc, nome_enc))) =
            sqlx::query_as::<_, (u64, String)>("SELECT id, name FROM SESSAO_RPG WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await
        {
            return Ok((Some(id_enc), nome_enc));
        }
    }

    if let Some(nome) = sessao_nome.map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(Some((id_enc, nome_enc))) = sqlx::query_as::<_, (u64, String)>(
            "SELECT id, name FROM SESSAO_RPG WHERE name = ? LIMIT 1",
        )
        .bind(nome)
        .fetch_optional(pool)
        .await
        {
            return Ok((Some(id_enc), nome_enc));
        }
        return Ok((sessao_id, nome.chars().take(255).collect()));
    }

    if let Some(convite) = codigo_convite.map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(id_convite) = convite.parse::<u64>() {
            if let Ok(Some((id_enc, nome_enc))) =
                sqlx::query_as::<_, (u64, String)>("SELECT id, name FROM SESSAO_RPG WHERE id = ?")
                    .bind(id_convite)
                    .fetch_optional(pool)
                    .await
            {
                return Ok((Some(id_enc), nome_enc));
            }
        }
        if let Ok(Some((id_enc, nome_enc))) = sqlx::query_as::<_, (u64, String)>(
            "SELECT id, name FROM SESSAO_RPG WHERE name = ? LIMIT 1",
        )
        .bind(convite)
        .fetch_optional(pool)
        .await
        {
            return Ok((Some(id_enc), nome_enc));
        }
        return Ok((None, convite.chars().take(255).collect()));
    }

    Ok((None, "Crônicas de Kharos".to_string()))
}

async fn solicitar_verificacao(
    State(state): State<VerifierState>,
    ConnectInfo(par): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Corpo(req): Corpo<SolicitarVerificacaoRequest>,
) -> Result<(StatusCode, Json<SolicitarVerificacaoResponse>), VerifierError> {
    let ip = ip_cliente(par.ip(), &headers, &state.proxies_confiaveis);
    let chave_solicitar = chave_ip("cadastro", ip);
    if let Some(ate) = seguranca::bloqueio(&state.pool, &chave_solicitar).await? {
        return Err(VerifierError::TooManyRequests {
            aguarde_segundos: segundos_ate(ate),
        });
    }

    let handle_original = req
        .nome
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| req.login.trim())
        .to_string();
    let login = req.login.trim().to_lowercase();
    let email = req.email.trim().to_lowercase();

    let papel = req
        .papel
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Mestre (Host)")
        .to_string();

    let sessao_sistema: String = req
        .sessao_sistema
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Old Dragon / D&D 5e")
        .chars()
        .take(100)
        .collect();

    let sessao_descricao = req
        .sessao_descricao
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(1000).collect::<String>());

    let servidor = req
        .servidor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("127.0.0.1:8080")
        .to_string();

    validar_dados_cadastro(
        &login,
        &handle_original,
        &email,
        &req.senha,
        &papel,
        &servidor,
    )?;

    let codigo = match req
        .codigo
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(c) => {
            validar_codigo_customizado(c)?;
            c.to_string()
        }
        None => gerar_codigo_6_digitos(),
    };

    let token_customizado = match req
        .token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(t) => {
            validar_token_customizado(t)?;
            Some(t.to_string())
        }
        None => None,
    };

    let ja_existe: Option<(u64,)> =
        sqlx::query_as("SELECT id FROM USUARIOS WHERE login = ? AND ativo = TRUE")
            .bind(&login)
            .fetch_optional(&state.pool)
            .await?;
    if ja_existe.is_some() {
        return Err(VerifierError::Conflict(
            "Este login já está em uso.".to_string(),
        ));
    }

    let (sessao_id, sessao_nome) = resolver_sessao(
        &state.pool,
        req.sessao_id,
        req.sessao_nome.as_deref(),
        req.codigo_convite.as_deref(),
    )
    .await?;

    let senha_hash = gerar_hash_senha(req.senha).await?;
    let expira_em = Utc::now() + Duration::hours(VALIDADE_VERIFICACAO_HORAS);

    // Invalida códigos pendentes anteriores do mesmo login para evitar múltiplos códigos ativos.
    sqlx::query("DELETE FROM VERIFICACOES_CADASTRO WHERE login = ? AND confirmado = FALSE")
        .bind(&login)
        .execute(&state.pool)
        .await?;

    let mut token_final = String::new();
    for tentativa in 0..5 {
        let sufixo_base = req
            .codigo_convite
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(Some(sessao_nome.as_str()));
        let candidato = match (&token_customizado, tentativa) {
            (Some(t), 0) => t.clone(),
            _ => gerar_token_ativacao(sufixo_base),
        };

        let res = sqlx::query(
            "INSERT INTO VERIFICACOES_CADASTRO \
             (token, codigo, login, nome, email, senha_hash, papel, sessao_id, sessao_nome, sessao_sistema, sessao_descricao, codigo_convite, servidor, expira_em) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&candidato)
        .bind(&codigo)
        .bind(&login)
        .bind(&handle_original)
        .bind(&email)
        .bind(&senha_hash)
        .bind(&papel)
        .bind(sessao_id)
        .bind(&sessao_nome)
        .bind(&sessao_sistema)
        .bind(&sessao_descricao)
        .bind(&req.codigo_convite)
        .bind(&servidor)
        .bind(expira_em)
        .execute(&state.pool)
        .await;

        match res {
            Ok(_) => {
                token_final = candidato;
                break;
            }
            Err(err)
                if err
                    .as_database_error()
                    .is_some_and(|d| d.is_unique_violation()) =>
            {
                if tentativa == 4 {
                    return Err(VerifierError::Conflict(
                        "Não foi possível alocar um token único. Tente novamente.".to_string(),
                    ));
                }
            }
            Err(err) => return Err(VerifierError::from(err)),
        }
    }

    seguranca::registrar(&state.pool, &chave_solicitar, SOLICITACOES_LIVRES_POR_IP).await?;

    let verify_url = format!("{}/verify?token={token_final}", state.public_base_url);
    tracing::info!(
        "Verificação solicitada para '{login}' ({email}) a partir de {ip}: token={token_final}"
    );

    let mut email_enviado = false;
    if req.enviar_email.unwrap_or(true) {
        if let Some(smtp) = &state.smtp {
            match crate::email::enviar_email_verificacao(
                smtp,
                &email,
                &handle_original,
                &sessao_nome,
                &servidor,
                &codigo,
                &token_final,
                &verify_url,
            )
            .await
            {
                Ok(()) => {
                    email_enviado = true;
                }
                Err(err) => {
                    tracing::error!("Falha ao enviar e-mail SMTP para '{email}': {err}");
                }
            }
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(SolicitarVerificacaoResponse {
            token: token_final,
            codigo,
            verify_url,
            login,
            nome: handle_original,
            email,
            papel,
            sessao_nome,
            servidor,
            email_enviado,
            expira_em,
        }),
    ))
}

/// Efetiva o cadastro de forma atômica (transação SQL):
/// 1. Cria ou ativa o usuário em `USUARIOS` (`email_verificado = TRUE`)
/// 2. Vincula o jogador em `SESSAO_PLAYERS` (quando `sessao_id` existir)
/// 3. Marca `VERIFICACOES_CADASTRO.confirmado = TRUE`
async fn efetivar_cadastro_no_banco(
    pool: &MySqlPool,
    verif: &VerificacaoRow,
) -> Result<u64, VerifierError> {
    if verif.confirmado {
        if let Some(uid) = verif.usuario_id {
            return Ok(uid);
        }
    }

    let mut tx = pool.begin().await?;

    let existente: Option<(u64, bool)> =
        sqlx::query_as("SELECT id, email_verificado FROM USUARIOS WHERE login = ? FOR UPDATE")
            .bind(&verif.login)
            .fetch_optional(&mut *tx)
            .await?;

    let usuario_id = match existente {
        Some((id, false)) => {
            sqlx::query(
                "UPDATE USUARIOS SET nome = ?, email = ?, senha_hash = ?, ativo = TRUE, email_verificado = TRUE WHERE id = ?",
            )
            .bind(&verif.nome)
            .bind(&verif.email)
            .bind(&verif.senha_hash)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            id
        }
        Some((id, true)) => {
            // Conta já verificada anteriormente: apenas retorna o ID sem sobrescrever a senha de uma conta já ativa!
            id
        }
        None => {
            let res = sqlx::query(
                "INSERT INTO USUARIOS (login, nome, email, senha_hash, admin, ativo, email_verificado) \
                 VALUES (?, ?, ?, ?, FALSE, TRUE, TRUE)",
            )
            .bind(&verif.login)
            .bind(&verif.nome)
            .bind(&verif.email)
            .bind(&verif.senha_hash)
            .execute(&mut *tx)
            .await?;
            res.last_insert_id()
        }
    };

    let sessao_id_final = match verif.sessao_id {
        Some(sid) => sid,
        None => {
            let sessao_existente: Option<(u64,)> =
                sqlx::query_as("SELECT id FROM SESSAO_RPG WHERE name = ? ORDER BY id DESC LIMIT 1")
                    .bind(&verif.sessao_nome)
                    .fetch_optional(&mut *tx)
                    .await?;

            match sessao_existente {
                Some((sid,)) => sid,
                None => {
                    let descricao = verif.sessao_descricao.clone().unwrap_or_else(|| {
                        format!(
                            "Sessão criada pelo Mestre '{}' ({}) no servidor {}",
                            verif.nome, verif.login, verif.servidor
                        )
                    });
                    let res_sessao = sqlx::query(
                        "INSERT INTO SESSAO_RPG (name, description, system) VALUES (?, ?, ?)",
                    )
                    .bind(&verif.sessao_nome)
                    .bind(&descricao)
                    .bind(&verif.sessao_sistema)
                    .execute(&mut *tx)
                    .await?;
                    res_sessao.last_insert_id()
                }
            }
        }
    };

    let ja_na_sessao: Option<(u64,)> = sqlx::query_as(
        "SELECT id FROM SESSAO_PLAYERS WHERE sessao_id = ? AND player_name = ? LIMIT 1",
    )
    .bind(sessao_id_final)
    .bind(&verif.login)
    .fetch_optional(&mut *tx)
    .await?;

    if ja_na_sessao.is_none() {
        sqlx::query(
            "INSERT INTO SESSAO_PLAYERS (sessao_id, name, player_name, resume) VALUES (?, ?, ?, ?)",
        )
        .bind(sessao_id_final)
        .bind(&verif.nome)
        .bind(&verif.login)
        .bind(format!(
            "{} confirmado via verificação de e-mail",
            verif.papel
        ))
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE VERIFICACOES_CADASTRO SET confirmado = TRUE, usuario_id = ?, sessao_id = ?, confirmado_em = ? WHERE id = ?",
    )
    .bind(usuario_id)
    .bind(sessao_id_final)
    .bind(Utc::now())
    .bind(verif.id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let _ = verif.codigo_convite.as_deref();
    tracing::info!(
        "Operador '{}' (id={usuario_id}) confirmado e gravado no MariaDB!",
        verif.login
    );

    Ok(usuario_id)
}

/// Busca uma verificação:
/// - Por `token` completo (ex: `ACT-98-7741-KHR`): busca direta na coluna `token`.
/// - Por `codigo` de 6 dígitos (ex: `482910`): exige `login` ou `email` se informado; na web manual,
///   permite código de 6 dígitos apenas se ainda não expirado e único.
async fn buscar_verificacao(
    pool: &MySqlPool,
    token: Option<&str>,
    codigo: Option<&str>,
    login: Option<&str>,
    email: Option<&str>,
) -> Result<Option<VerificacaoRow>, VerifierError> {
    if let Some(t) = token.map(str::trim).filter(|s| !s.is_empty()) {
        // Se o valor passado em `token` for na verdade um código numérico de 6 dígitos, trata como código.
        let eh_codigo_6_digitos = t.len() == 6 && t.chars().all(|c| c.is_ascii_digit());
        if !eh_codigo_6_digitos {
            let por_token: Option<VerificacaoRow> = sqlx::query_as::<_, VerificacaoRow>(
                "SELECT id, token, login, nome, email, senha_hash, papel, sessao_id, sessao_nome, \
                 sessao_sistema, sessao_descricao, codigo_convite, servidor, confirmado, usuario_id, expira_em \
                 FROM VERIFICACOES_CADASTRO WHERE token = ? ORDER BY id DESC LIMIT 1",
            )
            .bind(t)
            .fetch_optional(pool)
            .await?;
            if por_token.is_some() {
                return Ok(por_token);
            }
        }
    }

    let codigo_busca = codigo
        .or(token)
        .map(str::trim)
        .filter(|s| s.len() == 6 && s.chars().all(|c| c.is_ascii_digit()));

    if let Some(c) = codigo_busca {
        if let Some(l) = login.map(str::trim).filter(|s| !s.is_empty()) {
            return Ok(sqlx::query_as::<_, VerificacaoRow>(
                "SELECT id, token, login, nome, email, senha_hash, papel, sessao_id, sessao_nome, \
                 sessao_sistema, sessao_descricao, codigo_convite, servidor, confirmado, usuario_id, expira_em \
                 FROM VERIFICACOES_CADASTRO WHERE codigo = ? AND login = ? ORDER BY id DESC LIMIT 1",
            )
            .bind(c)
            .bind(l.to_lowercase())
            .fetch_optional(pool)
            .await?);
        }

        if let Some(em) = email.map(str::trim).filter(|s| !s.is_empty()) {
            return Ok(sqlx::query_as::<_, VerificacaoRow>(
                "SELECT id, token, login, nome, email, senha_hash, papel, sessao_id, sessao_nome, \
                 sessao_sistema, sessao_descricao, codigo_convite, servidor, confirmado, usuario_id, expira_em \
                 FROM VERIFICACOES_CADASTRO WHERE codigo = ? AND email = ? ORDER BY id DESC LIMIT 1",
            )
            .bind(c)
            .bind(em.to_lowercase())
            .fetch_optional(pool)
            .await?);
        }
    }

    Ok(None)
}

async fn confirmar_verificacao_json(
    State(state): State<VerifierState>,
    ConnectInfo(par): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Corpo(req): Corpo<ConfirmarVerificacaoRequest>,
) -> Result<Json<ConfirmacaoResponse>, VerifierError> {
    let ip = ip_cliente(par.ip(), &headers, &state.proxies_confiaveis);
    let mut chaves = vec![chave_ip("ip", ip)];
    if let Some(l) = req
        .login
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        chaves.push(chave_conta_verif(&l.to_lowercase()));
    }

    if let Some(ate) = seguranca::maior_bloqueio(&state.pool, &chaves).await? {
        return Err(VerifierError::TooManyRequests {
            aguarde_segundos: segundos_ate(ate),
        });
    }

    let tem_token = req.token.as_deref().is_some_and(|t| {
        !t.trim().is_empty()
            && !(t.trim().len() == 6 && t.trim().chars().all(|c| c.is_ascii_digit()))
    });
    let tem_identificador = req.login.as_deref().is_some_and(|l| !l.trim().is_empty())
        || req.email.as_deref().is_some_and(|e| !e.trim().is_empty());

    if !tem_token && !tem_identificador {
        return Err(VerifierError::BadRequest(
            "Para confirmar com o código de 6 dígitos, informe também o 'login' ou 'email' da conta.".into(),
        ));
    }

    let verif = match buscar_verificacao(
        &state.pool,
        req.token.as_deref(),
        req.codigo.as_deref(),
        req.login.as_deref(),
        req.email.as_deref(),
    )
    .await?
    {
        Some(v) => v,
        None => {
            for chave in &chaves {
                let _ = seguranca::registrar(&state.pool, chave, 2).await;
            }
            return Err(VerifierError::NotFound(
                "Token ou código de confirmação inválido.".into(),
            ));
        }
    };

    if !verif.confirmado && verif.expira_em < Utc::now() {
        return Err(VerifierError::BadRequest(
            "Este código/token de verificação expirou. Solicite um novo envio.".into(),
        ));
    }

    let usuario_id = efetivar_cadastro_no_banco(&state.pool, &verif).await?;
    let _ = seguranca::limpar(&state.pool, &chave_conta_verif(&verif.login)).await;

    Ok(Json(ConfirmacaoResponse {
        confirmado: true,
        usuario_id,
        token: verif.token,
        login: verif.login,
        handle: verif.nome,
        email: verif.email,
        papel: verif.papel,
        sessao_nome: verif.sessao_nome,
        servidor: verif.servidor,
        status_conexao: "Ativo e Sincronizado".to_string(),
    }))
}

async fn status_verificacao_json(
    State(state): State<VerifierState>,
    Consulta(q): Consulta<VerifyQuery>,
) -> Result<Json<ConfirmacaoResponse>, VerifierError> {
    let verif = buscar_verificacao(
        &state.pool,
        q.token.as_deref(),
        q.code.as_deref(),
        q.login.as_deref(),
        q.email.as_deref(),
    )
    .await?
    .ok_or_else(|| VerifierError::NotFound("Verificação não encontrada.".into()))?;

    Ok(Json(ConfirmacaoResponse {
        confirmado: verif.confirmado,
        usuario_id: verif.usuario_id.unwrap_or(0),
        token: verif.token,
        login: verif.login,
        handle: verif.nome,
        email: verif.email,
        papel: verif.papel,
        sessao_nome: verif.sessao_nome,
        servidor: verif.servidor,
        status_conexao: if verif.confirmado {
            "Ativo e Sincronizado".to_string()
        } else {
            "Aguardando Confirmação".to_string()
        },
    }))
}

fn escapar_html(texto: &str) -> String {
    texto
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[allow(clippy::too_many_arguments)]
fn renderizar_pagina(
    modo: &str,
    titulo: &str,
    subtitulo: &str,
    handle: &str,
    papel: &str,
    sessao: &str,
    servidor: &str,
    status_conexao: &str,
    rodape: &str,
    window_status: &str,
) -> Html<String> {
    let html = HTML_TEMPLATE
        .replace("<body>", &format!("<body data-mode=\"{modo}\">"))
        .replace("{{HEADING_TITLE}}", &escapar_html(titulo))
        .replace("{{HEADING_SUBTITLE}}", &escapar_html(subtitulo))
        .replace("{{HANDLE_USUARIO}}", &escapar_html(handle))
        .replace("{{PAPEL_MESA}}", &escapar_html(papel))
        .replace("{{SESSAO_CAMPANHA}}", &escapar_html(sessao))
        .replace("{{SERVIDOR_DESTINO}}", &escapar_html(servidor))
        .replace("{{STATUS_CONEXAO}}", &escapar_html(status_conexao))
        .replace("{{FOOTER_MESSAGE}}", &escapar_html(rodape))
        .replace("{{WINDOW_STATUS}}", &escapar_html(window_status));

    Html(html)
}

pub async fn pagina_verificacao_html(
    State(state): State<VerifierState>,
    ConnectInfo(par): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Consulta(q): Consulta<VerifyQuery>,
) -> Html<String> {
    let chave_busca = q
        .token
        .as_deref()
        .or(q.code.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let Some(chave) = chave_busca else {
        return renderizar_pagina(
            "manual",
            "VERIFICAÇÃO DE CREDENCIAL",
            "Insira abaixo o seu Token recebido por e-mail para ativar sua conta.",
            "—",
            "Jogador",
            "—",
            "127.0.0.1:8080",
            "Aguardando Token de Ativação",
            "Informe o token de confirmação e clique em [ Validar Credencial ↵].",
            "Aguardando entrada do operador.",
        );
    };

    let ip = ip_cliente(par.ip(), &headers, &state.proxies_confiaveis);
    let chave_ip_verif = chave_ip("ip", ip);
    if let Ok(Some(ate)) = seguranca::bloqueio(&state.pool, &chave_ip_verif).await {
        let segs = segundos_ate(ate);
        return renderizar_pagina(
            "error",
            "MUITAS TENTATIVAS DE VERIFICAÇÃO ✖",
            &format!("Por segurança, aguarde {segs}s antes de tentar validar outro código."),
            "—",
            "—",
            "—",
            "127.0.0.1:8080",
            "Temporariamente Bloqueado (Rate-Limit)",
            "Aguarde o tempo de segurança antes de tentar novamente.",
            "Bloqueio temporário ativo.",
        );
    }

    match buscar_verificacao(
        &state.pool,
        Some(chave),
        Some(chave),
        q.login.as_deref(),
        q.email.as_deref(),
    )
    .await
    {
        Ok(Some(verif)) => {
            if !verif.confirmado && verif.expira_em < Utc::now() {
                return renderizar_pagina(
                    "error",
                    "TOKEN DE ATIVAÇÃO EXPIRADO ✖",
                    "O prazo de validade desta credencial expirou. Solicite um novo código no aplicativo desktop.",
                    &verif.nome,
                    &verif.papel,
                    &verif.sessao_nome,
                    &verif.servidor,
                    "Expirado — Não Sincronizado",
                    "Retorne ao aplicativo desktop do RPGASYNCST e clique em [ Reenviar E-mail ].",
                    "Erro: Token expirado.",
                );
            }

            if let Err(err) = efetivar_cadastro_no_banco(&state.pool, &verif).await {
                tracing::error!("Erro ao efetivar cadastro na verificação web: {err:?}");
                return renderizar_pagina(
                    "error",
                    "ERRO AO SINCRONIZAR NO BANCO ✖",
                    "Não foi possível concluir a gravação da sua credencial no nó principal.",
                    &verif.nome,
                    &verif.papel,
                    &verif.sessao_nome,
                    &verif.servidor,
                    "Falha na Sincronização",
                    "Verifique se o login já está em uso ou tente novamente pelo aplicativo desktop.",
                    "Erro ao gravar no banco de dados.",
                );
            }

            let _ = seguranca::limpar(&state.pool, &chave_conta_verif(&verif.login)).await;

            renderizar_pagina(
                "ok",
                "ACESSO AUTORIZADO COM SUCESSO! ✔",
                "Operador, sua credencial foi confirmada e sincronizada com o nó principal da mesa!",
                &verif.nome,
                &verif.papel,
                &verif.sessao_nome,
                &verif.servidor,
                "Ativo e Sincronizado",
                "Seu cadastro foi validado. Você já pode fechar esta janela e retornar ao aplicativo desktop do RPGASYNCST.",
                "Carregado com sucesso.",
            )
        }
        Ok(None) | Err(_)
            if state.allow_demo_token && chave.eq_ignore_ascii_case("ACT-98-7741-KHR") =>
        {
            renderizar_pagina(
                "ok",
                "ACESSO AUTORIZADO COM SUCESSO! ✔",
                "Operador, sua credencial foi confirmada e sincronizada com o nó principal da mesa!",
                "Kaelen",
                "Jogador",
                "Crônicas de Kharos",
                "127.0.0.1:8080",
                "Ativo e Sincronizado",
                "Seu cadastro foi validado. Você já pode fechar esta janela e retornar ao aplicativo desktop do RPGASYNCST.",
                "Carregado com sucesso.",
            )
        }
        Ok(None) => {
            let _ = seguranca::registrar(&state.pool, &chave_ip_verif, 3).await;
            renderizar_pagina(
                "error",
                "CREDENCIAL NÃO ENCONTRADA ✖",
                "O token informado não foi localizado no nó principal da mesa.",
                "Desconhecido",
                "—",
                "—",
                "127.0.0.1:8080",
                "Token Inválido",
                "Verifique o link recebido por e-mail ou digite o token novamente abaixo.",
                "Erro: Token não localizado.",
            )
        }
        Err(err) => {
            tracing::error!("Falha ao consultar banco na verificação web: {err:?}");
            renderizar_pagina(
                "error",
                "FALHA DE COMUNICAÇÃO COM O NÓ ✖",
                "O banco de dados principal está temporariamente indisponível.",
                "—",
                "—",
                "—",
                "127.0.0.1:8080",
                "Sem Conexão com o Banco",
                "Tente recarregar a página em alguns instantes.",
                "Erro de comunicação.",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gera_token_no_padrao_esperado() {
        let token = gerar_token_ativacao(Some("KHAROS-98"));
        assert!(token.starts_with("ACT-98-"));
        assert!(token.ends_with("-KHR"));
        assert_eq!(token.len(), 15);
    }

    #[test]
    fn gera_codigo_de_6_digitos_numericos() {
        for _ in 0..20 {
            let codigo = gerar_codigo_6_digitos();
            assert_eq!(codigo.len(), 6);
            assert!(codigo.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn valida_dados_de_cadastro_corretamente() {
        assert!(validar_dados_cadastro(
            "kaelen",
            "Kaelen",
            "kaelen@chronos.local",
            "senha-segura-96",
            "Jogador",
            "127.0.0.1:8080"
        )
        .is_ok());

        // Login inválido
        assert!(validar_dados_cadastro(
            "ab",
            "Kaelen",
            "kaelen@chronos.local",
            "senha-segura-96",
            "Jogador",
            "127.0.0.1:8080"
        )
        .is_err());

        // Senha igual ao login
        assert!(validar_dados_cadastro(
            "kaelen",
            "Kaelen",
            "kaelen@chronos.local",
            "Kaelen",
            "Jogador",
            "127.0.0.1:8080"
        )
        .is_err());

        // E-mail malformado
        assert!(validar_dados_cadastro(
            "kaelen",
            "Kaelen",
            "email-invalido@",
            "senha-segura-96",
            "Jogador",
            "127.0.0.1:8080"
        )
        .is_err());
    }

    #[test]
    fn escapa_caracteres_html_contra_xss() {
        let entrada = "<script>alert(\"xss\" & '1')</script>";
        assert_eq!(
            escapar_html(entrada),
            "&lt;script&gt;alert(&quot;xss&quot; &amp; &#39;1&#39;)&lt;/script&gt;"
        );
    }
}
