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

1. Crie uma **nova** migration em `migrations/` (ex.: `0002_nova_tabela.sql`). Nunca edite uma migration já aplicada: o checksum muda e a API deixa de subir.
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

## 4. Comportamento da API

A API é só um CRUD por tabela para desacoplar o cliente do banco: **não tem regra de negócio**. As únicas validações são as do próprio MariaDB (tipos, tamanhos e chaves estrangeiras).

| Situação | Status | Corpo |
| :--- | :---: | :--- |
| Sucesso na leitura/alteração | `200` | Registro ou lista em JSON |
| Registro criado | `201` | Registro criado |
| Registro apagado | `204` | Vazio |
| `id` inexistente | `404` | `{"error": "..."}` |
| JSON com tipo/campo errado | `422` | Texto gerado pelo Axum |
| Texto maior que a coluna ou dado em formato inválido | `400` | `{"error": "..."}` |
| Chave estrangeira inexistente (ex.: `sessao_id` que não existe) | `409` | `{"error": "..."}` |
| Falha inesperada no banco | `500` | `{"error": "Erro interno ao acessar o banco de dados."}` (detalhe só no log) |

- **Atualização parcial (`PUT`)**: campos ausentes ou `null` mantêm o valor atual. Por isso, hoje não é possível limpar uma coluna opcional (gravar `NULL`) pela API.
- **Exclusão em cascata**: apagar uma `SESSAO_RPG` apaga players, assets e fichas dela; apagar um `SESSAO_PLAYERS` apaga as fichas dele (chaves estrangeiras `ON DELETE CASCADE`).
- **`GET /health`**: responde `200` quando a API e o MariaDB estão acessíveis e `503` quando o banco não responde.
- **Schema**: as tabelas são criadas **somente** pelas migrations desta pasta, aplicadas automaticamente quando a API sobe. Nenhum outro repositório deve rodar `sqlx migrate run` neste banco, porque as migrations compartilham a tabela `_sqlx_migrations` e a API deixaria de subir.

---

## 5. Configuração e Execução Local

Variáveis de ambiente (ou arquivo `.env`, veja [`.env.example`](.env.example)):

| Variável | Obrigatória | Padrão |
| :--- | :---: | :--- |
| `DATABASE_URL` | **Sim** | Sem padrão; a API não sobe sem ela. Caracteres especiais da senha devem ser codificados na URL (`#` vira `%23`, `@` vira `%40`). |
| `API_HOST` | Não | `0.0.0.0` |
| `API_PORT` | Não | `8080` |
| `RUST_LOG` | Não | `rpg_db_crud=info,tower_http=info` |

```bash
cargo run
```

A imagem Docker usa Rust 1.88 (mínimo exigido pelas dependências do `Cargo.lock`) e compila com `--locked`.

---

## 6. Cliente na Aplicação `.exe` (Iced)

O cliente oficial fica no repositório **AsyncTTRPG96**, módulo `src/api/` (`RpgApiClient`, com modelos tipados das 4 tabelas). Ao adicionar uma tabela ou coluna aqui, atualize também `src/api/models.rs` lá.
