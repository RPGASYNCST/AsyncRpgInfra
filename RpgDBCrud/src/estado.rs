use axum::extract::FromRef;
use ipnet::IpNet;
use sqlx::MySqlPool;
use std::sync::Arc;

/// Estado compartilhado pelos handlers. Handlers que só precisam do banco continuam
/// pedindo `State<MySqlPool>` (via `FromRef`).
#[derive(Clone)]
pub struct AppState {
    pub pool: MySqlPool,
    pub auth: Arc<ConfigAuth>,
}

impl FromRef<AppState> for MySqlPool {
    fn from_ref(estado: &AppState) -> Self {
        estado.pool.clone()
    }
}

#[derive(Debug, Clone)]
pub struct ConfigAuth {
    /// Validade do token de sessão entregue no login.
    pub duracao_sessao: chrono::Duration,
    /// Proxies reversos confiáveis (ex.: o Nginx Proxy Manager). Só deles a API aceita
    /// `X-Forwarded-For`/`X-Real-IP` para descobrir o IP real do cliente.
    pub proxies_confiaveis: Vec<IpNet>,
}
