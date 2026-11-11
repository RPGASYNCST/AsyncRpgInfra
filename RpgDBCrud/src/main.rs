mod error;
mod routes;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::MySqlPool;
use std::env;
use std::time::Duration;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rpg_db_crud=info,tower_http=info".into()),
        )
        .init();

    let database_url = env::var("DATABASE_URL")
        .map_err(|_| "A variável de ambiente DATABASE_URL é obrigatória (veja .env.example).")?;

    let host = env::var("API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("API_PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("{host}:{port}");

    tracing::info!("Conectando ao MariaDB...");
    let pool = MySqlPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await?;

    tracing::info!("Executando migrations das tabelas...");
    sqlx::migrate!("./migrations").run(&pool).await?;

    let app = Router::new()
        .route("/health", get(health_check))
        .nest("/api", routes::api_router())
        .layer(TraceLayer::new_for_http())
        .with_state(pool.clone());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Servidor RpgDBCrud rodando em http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    pool.close().await;
    tracing::info!("Servidor RpgDBCrud encerrado.");
    Ok(())
}

/// Responde 200 quando a API e o MariaDB estão acessíveis e 503 quando o banco não responde.
async fn health_check(State(pool): State<MySqlPool>) -> (StatusCode, Json<Value>) {
    match sqlx::query("SELECT 1").execute(&pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({ "status": "ok", "service": "RpgDBCrud", "database": "ok" })),
        ),
        Err(err) => {
            tracing::error!("Health-check: MariaDB indisponível: {err}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(
                    json!({ "status": "erro", "service": "RpgDBCrud", "database": "indisponivel" }),
                ),
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
    tracing::info!("Sinal de encerramento recebido; finalizando requisições em andamento...");
}
