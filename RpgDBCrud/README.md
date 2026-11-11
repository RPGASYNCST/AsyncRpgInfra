# RpgDBCrud — API REST para o Cliente `.exe` (Rust + Axum + MariaDB)

API REST leve em Rust (`axum` + `sqlx`) para que o executável `.exe` da aplicação (Iced) faça todas as operações de **CRUD** via HTTP/JSON sem precisar expor credenciais ou acesso direto ao banco MariaDB.

---

## 1. Estrutura da Pasta `RpgDBCrud`

```text
RpgDBCrud/
├── Cargo.toml
├── Dockerfile
├── .env.example
├── migrations/
│   └── 0001_init_tables.sql       # Cria as 4 tabelas automaticamente no MariaDB ao iniciar
└── src/
    ├── main.rs                    # Conecta no MariaDB, roda migrations e sobe a API na porta 8080
    ├── error.rs                   # Padroniza respostas de erro em JSON ({ "error": "..." })
    └── routes/
        ├── mod.rs                 # Registra os CRUDs (1 linha por tabela)
        ├── sessao_rpg.rs          # CRUD da tabela SESSAO_RPG
        ├── sessao_assets.rs       # CRUD da tabela SESSAO_ASSETS
        ├── sessao_players.rs      # CRUD da tabela SESSAO_PLAYERS
        └── players_sheets.rs      # CRUD da tabela PLAYERS_SHEETS
```

---

## 2. Rotas Disponíveis (1 CRUD por Tabela)

Todas as tabelas seguem exatamente o mesmo padrão de 5 endpoints:

| Tabela no MariaDB | Rota Base | Operações Disponíveis |
| :--- | :--- | :--- |
| **`SESSAO_RPG`** | `/api/sessao_rpg` | `GET /`, `GET /{id}`, `POST /`, `PUT /{id}`, `DELETE /{id}` |
| **`SESSAO_ASSETS`** | `/api/sessao_assets` | `GET /` *(ou `?sessao_id=1`)*, `GET /{id}`, `POST /`, `PUT /{id}`, `DELETE /{id}` |
| **`SESSAO_PLAYERS`** | `/api/sessao_players` | `GET /` *(ou `?sessao_id=1`)*, `GET /{id}`, `POST /`, `PUT /{id}`, `DELETE /{id}` |
| **`PLAYERS_SHEETS`** | `/api/players_sheets` | `GET /` *(ou `?sessao_id=1&players_id=2`)*, `GET /{id}`, `POST /`, `PUT /{id}`, `DELETE /{id}` |

---

## 3. Como Adicionar um Novo CRUD de Tabela

1. Adicione o `CREATE TABLE` em `migrations/` (ou uma nova migration `0002_nova_tabela.sql`).
2. Duplique qualquer arquivo de `src/routes/` (por exemplo, copie `sessao_rpg.rs` para `src/routes/nova_tabela.rs`) e troque os campos da struct e a query SQL.
3. Abra [`src/routes/mod.rs`](src/routes/mod.rs) e adicione **1 linha**:
   ```rust
   pub mod nova_tabela;

   pub fn api_router() -> Router<MySqlPool> {
       Router::new()
           // ...
           .nest("/nova_tabela", nova_tabela::router())
   }
   ```

---

## 4. Como Chamar a API no Código da sua Aplicação `.exe` (Iced)

No `Cargo.toml` da sua aplicação `.exe` (Iced), adicione:
```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
chrono = { version = "0.4", features = ["serde"] }
```

E crie um arquivo `src/api.rs` na sua aplicação `.exe` copiando o cliente pronto abaixo:

```rust
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct RpgApiClient {
    client: Client,
    base_url: String,
}

// ==================== MODELS ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessaoRpg {
    pub id: u64,
    pub name: String,
    pub description: Option<String>,
    pub system: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessaoRpg {
    pub name: String,
    pub description: Option<String>,
    pub system: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessaoAsset {
    pub id: u64,
    pub sessao_id: u64,
    pub glob: String,
    pub description: Option<String>,
    pub resume: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessaoAsset {
    pub sessao_id: u64,
    pub glob: String,
    pub description: Option<String>,
    pub resume: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessaoPlayer {
    pub id: u64,
    pub sessao_id: u64,
    pub name: String,
    pub player_name: String,
    pub resume: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessaoPlayer {
    pub sessao_id: u64,
    pub name: String,
    pub player_name: String,
    pub resume: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayersSheet {
    pub id: u64,
    pub sessao_id: u64,
    pub players_id: u64,
    pub system: String,
    pub sheet: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlayersSheet {
    pub sessao_id: u64,
    pub players_id: u64,
    pub system: String,
    pub sheet: Value,
}

// ==================== CLIENTE HTTP GENÉRICO ====================

impl RpgApiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// GET /api/{resource}
    pub async fn list<T: DeserializeOwned>(&self, resource: &str) -> Result<Vec<T>, String> {
        let url = format!("{}/api/{resource}", self.base_url);
        self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json::<Vec<T>>()
            .await
            .map_err(|e| e.to_string())
    }

    /// GET /api/{resource}/{id}
    pub async fn get_by_id<T: DeserializeOwned>(&self, resource: &str, id: u64) -> Result<T, String> {
        let url = format!("{}/api/{resource}/{id}", self.base_url);
        self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json::<T>()
            .await
            .map_err(|e| e.to_string())
    }

    /// POST /api/{resource}
    pub async fn create<B: Serialize, T: DeserializeOwned>(
        &self,
        resource: &str,
        body: &B,
    ) -> Result<T, String> {
        let url = format!("{}/api/{resource}", self.base_url);
        self.client
            .post(&url)
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json::<T>()
            .await
            .map_err(|e| e.to_string())
    }

    /// PUT /api/{resource}/{id}
    pub async fn update<B: Serialize, T: DeserializeOwned>(
        &self,
        resource: &str,
        id: u64,
        body: &B,
    ) -> Result<T, String> {
        let url = format!("{}/api/{resource}/{id}", self.base_url);
        self.client
            .put(&url)
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json::<T>()
            .await
            .map_err(|e| e.to_string())
    }

    /// DELETE /api/{resource}/{id}
    pub async fn delete(&self, resource: &str, id: u64) -> Result<(), String> {
        let url = format!("{}/api/{resource}/{id}", self.base_url);
        self.client
            .delete(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
```

### Exemplo de uso dentro de um `Task::perform` no Iced:
```rust
let api = RpgApiClient::new("http://SEU_SERVIDOR:8080");

// Listar todas as sessões:
let sessoes: Vec<SessaoRpg> = api.list("sessao_rpg").await?;

// Criar uma nova sessão:
let nova: SessaoRpg = api.create("sessao_rpg", &CreateSessaoRpg {
    name: "Campanha de Ordem Paranormal".into(),
    description: Some("Mesa de sexta-feira".into()),
    system: "Ordem Paranormal".into(),
}).await?;
```

