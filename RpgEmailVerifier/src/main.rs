mod email;
mod seguranca;
mod verificador;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use email::SmtpSettings;
use ipnet::IpNet;
use serde_json::{json, Value};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::MySqlPool;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

/// Limite máximo do corpo da requisição (64 KiB são mais que suficientes para payloads de cadastro).
const CORPO_MAX_BYTES: usize = 64 * 1024;
const REQUISICAO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct VerifierState {
    pub pool: MySqlPool,
    pub public_base_url: String,
    pub proxies_confiaveis: Arc<Vec<IpNet>>,
    pub allow_demo_token: bool,
    pub smtp: Option<Arc<SmtpSettings>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rpg_email_verifier=info,tower_http=info".into()),
        )
        .init();

    let database_url = env::var("DATABASE_URL")
        .map_err(|_| "A variável de ambiente DATABASE_URL é obrigatória (veja .env.example).")?;

    let host = env::var("VERIFIER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("VERIFIER_PORT").unwrap_or_else(|_| "8081".to_string());
    let addr = format!("{host}:{port}");

    let public_base_url = env::var("PUBLIC_VERIFY_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));

    let proxies_confiaveis =
        seguranca::ler_proxies(&env::var("TRUSTED_PROXIES").unwrap_or_default())?;

    let allow_demo_token = env::var("ALLOW_DEMO_TOKEN")
        .ok()
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"));

    tracing::info!("Inicializando pool MariaDB para RpgEmailVerifier...");
    let pool = MySqlPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(&database_url)?;

    if let Err(err) = verificador::garantir_schema_verificacao(&pool).await {
        tracing::warn!(
            "MariaDB ainda não acessível no startup ({err}); o servidor web continuará no ar e tentará conectar nas requisições."
        );
    }

    // Limpeza periódica de códigos de verificação vencidos e não confirmados.
    {
        let pool_limpeza = pool.clone();
        tokio::spawn(async move {
            let mut intervalo = tokio::time::interval(Duration::from_secs(3600));
            loop {
                intervalo.tick().await;
                if let Err(err) = verificador::limpar_verificacoes_expiradas(&pool_limpeza).await {
                    tracing::warn!("Falha na limpeza periódica de verificações expiradas: {err}");
                }
            }
        });
    }

    let state = VerifierState {
        pool: pool.clone(),
        public_base_url: public_base_url.trim_end_matches('/').to_string(),
        proxies_confiaveis: Arc::new(proxies_confiaveis),
        allow_demo_token,
        smtp: SmtpSettings::from_env().map(Arc::new),
    };

    let app = Router::new()
        .route("/", get(verificador::pagina_verificacao_html))
        .route("/health", get(health_check))
        .nest("/verify", verificador::router())
        .fallback(rota_inexistente)
        .layer(DefaultBodyLimit::max(CORPO_MAX_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUISICAO_TIMEOUT,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Servidor RpgEmailVerifier rodando em http://{addr}/verify");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    pool.close().await;
    tracing::info!("Servidor RpgEmailVerifier encerrado.");
    Ok(())
}

async fn rota_inexistente() -> verificador::VerifierError {
    verificador::VerifierError::NotFound("Rota não encontrada.".to_string())
}

async fn health_check(State(state): State<VerifierState>) -> (StatusCode, Json<Value>) {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({ "status": "ok", "service": "RpgEmailVerifier", "database": "ok" })),
        ),
        Err(err) => {
            tracing::error!("Health-check RpgEmailVerifier falhou: {err}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "status": "erro",
                    "service": "RpgEmailVerifier",
                    "database": "indisponivel"
                })),
            )
        }
    }
}

/// Aguarda Ctrl+C ou SIGTERM (enviado pelo `docker stop`) para encerrar sem cortar requisições.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!("Falha ao escutar Ctrl+C: {err}");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => {
                tracing::error!("Falha ao escutar SIGTERM: {err}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("Sinal de encerramento recebido no RpgEmailVerifier...");
}
