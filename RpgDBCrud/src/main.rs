mod auth;
mod error;
mod estado;
mod extract;
mod listagem;
mod routes;
mod validacao;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use axum::{Json, Router};
use estado::{AppState, ConfigAuth};
use serde_json::{json, Value};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::MySqlPool;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

/// Tamanho máximo do corpo de uma requisição (cobre fichas JSON de até 1 MB com folga).
const CORPO_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Tempo máximo de processamento de uma requisição.
const REQUISICAO_TIMEOUT: Duration = Duration::from_secs(30);

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

    let auth = ConfigAuth {
        duracao_sessao: chrono::Duration::hours(
            env_preenchida("SESSAO_HORAS")
                .map(|v| v.parse::<i64>())
                .transpose()
                .map_err(|_| "SESSAO_HORAS deve ser um número inteiro de horas.")?
                .unwrap_or(168),
        ),
        proxies_confiaveis: auth::ip::ler_proxies(
            &env::var("TRUSTED_PROXIES").unwrap_or_default(),
        )?,
    };
    if auth.proxies_confiaveis.is_empty() {
        tracing::warn!(
            "TRUSTED_PROXIES vazio: o IP usado na penalidade será o da conexão. Atrás do proxy reverso, configure o IP/rede dele para enxergar o IP real do cliente."
        );
    }

    if let (Some(login), Some(senha)) =
        (env_preenchida("ADMIN_LOGIN"), env_preenchida("ADMIN_SENHA"))
    {
        auth::garantir_admin_inicial(&pool, &login, &senha)
            .await
            .map_err(|e| format!("Falha ao criar o admin inicial: {e:?}"))?;
    }

    let estado = AppState {
        pool: pool.clone(),
        auth: Arc::new(auth),
    };

    // Limpeza periódica de sessões vencidas e penalidades expiradas.
    {
        let pool = pool.clone();
        tokio::spawn(async move {
            let mut intervalo = tokio::time::interval(Duration::from_secs(3600));
            loop {
                intervalo.tick().await;
                if let Err(err) = auth::limpar_expirados(&pool).await {
                    tracing::error!("Falha na limpeza periódica: {err}");
                }
            }
        });
    }

    let app = Router::new()
        .route("/health", get(health_check))
        .nest("/auth", auth::rotas::router(estado.clone()))
        .nest("/admin", auth::admin::router(estado.clone()))
        .nest(
            "/api",
            routes::api_router().route_layer(from_fn_with_state(
                estado.clone(),
                auth::middleware::exigir_login,
            )),
        )
        .fallback(rota_inexistente)
        .layer(DefaultBodyLimit::max(CORPO_MAX_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUISICAO_TIMEOUT,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(estado);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Servidor RpgDBCrud rodando em http://{addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    pool.close().await;
    tracing::info!("Servidor RpgDBCrud encerrado.");
    Ok(())
}

/// Variável de ambiente definida e não vazia (o compose repassa as opcionais vazias).
fn env_preenchida(nome: &str) -> Option<String> {
    env::var(nome).ok().filter(|v| !v.trim().is_empty())
}

async fn rota_inexistente() -> error::AppError {
    error::AppError::NotFound("Rota não encontrada.".to_string())
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
