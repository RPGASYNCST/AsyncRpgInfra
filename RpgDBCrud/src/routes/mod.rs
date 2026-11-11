pub mod players_sheets;
pub mod sessao_assets;
pub mod sessao_players;
pub mod sessao_rpg;

use axum::Router;
use sqlx::MySqlPool;

/// Registra todas as rotas CRUD da API (1 linha por tabela).
/// Para adicionar uma nova tabela no futuro:
/// 1. Crie `src/routes/nova_tabela.rs` copiando o padrão de qualquer uma abaixo.
/// 2. Adicione `pub mod nova_tabela;` no topo deste arquivo.
/// 3. Adicione `.nest("/nova_tabela", nova_tabela::router())` abaixo.
pub fn api_router() -> Router<MySqlPool> {
    Router::new()
        .nest("/sessao_rpg", sessao_rpg::router())
        .nest("/sessao_assets", sessao_assets::router())
        .nest("/sessao_players", sessao_players::router())
        .nest("/players_sheets", players_sheets::router())
}
