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

Todas as tabelas seguem exatamente o mesmo padrão de 5 endpoints: `GET /` (listagem paginada), `GET /{id}`, `POST /`, `PUT /{id}` e `DELETE /{id}`.

| Tabela no MariaDB | Rota Base | Filtros exatos | Busca `q` em | `order_by` aceita |
| :--- | :--- | :--- | :--- | :--- |
| **`SESSAO_RPG`** | `/api/sessao_rpg` | `system` | `name`, `description` | `id`, `name`, `system`, `created_at`, `updated_at` |
| **`SESSAO_ASSETS`** | `/api/sessao_assets` | `sessao_id` | `glob`, `description`, `resume` | `id`, `sessao_id`, `created_at`, `updated_at` |
| **`SESSAO_PLAYERS`** | `/api/sessao_players` | `sessao_id`, `player_name` | `name`, `player_name`, `resume` | `id`, `sessao_id`, `name`, `player_name`, `created_at`, `updated_at` |
| **`PLAYERS_SHEETS`** | `/api/players_sheets` | `sessao_id`, `players_id`, `system` | `system` | `id`, `sessao_id`, `players_id`, `system`, `created_at`, `updated_at` |

### 2.1. Listagem: paginação, busca, ordenação e datas

Parâmetros aceitos em todo `GET /api/{tabela}`, combináveis com os filtros exatos da tabela:

| Parâmetro | Exemplo | Efeito |
| :--- | :--- | :--- |
| `limit` | `limit=20` | Itens por página: 1 a 200, padrão 50 |
| `offset` | `offset=40` | Quantos itens pular |
| `q` | `q=dragao` | Busca parcial nas colunas de texto, sem diferenciar acentos nem maiúsculas |
| `order_by` | `order_by=name` | Coluna de ordenação (só as da tabela acima); padrão `id` |
| `order` | `order=asc` | `asc` ou `desc` (padrão) |
| `created_from` / `created_to` | `created_from=2026-11-01T00:00:00Z` | Intervalo de criação (RFC 3339) |
| `updated_from` | `updated_from=2026-11-11T18:00:00Z` | Só o que mudou desde a data, útil para sincronizar o cliente |

Exemplo: `GET /api/sessao_players?sessao_id=1&q=ana&order_by=name&order=asc&limit=20`

Resposta:

```json
{ "items": [ ... ], "total": 37, "limit": 20, "offset": 0 }
```

`total` é a quantidade de registros que atendem aos filtros, para montar a paginação no cliente. Nomes de coluna nunca vêm do cliente (só de listas fixas no código) e todo valor entra por *bind*, então os parâmetros não abrem espaço para SQL injection.

### 2.2. Validações de entrada

Cada tabela valida o corpo do `POST`/`PUT` antes de tocar no banco (`impl Create*/Update* { fn validar() }` em `src/routes/`), usando os auxiliares de [`src/validacao.rs`](src/validacao.rs):

| Validação | Onde é usada |
| :--- | :--- |
| Texto obrigatório: não vazio, sem espaços nas pontas, sem caracteres de controle, até N caracteres | `name` (255), `system` (100), `player_name` (255) |
| Texto livre opcional: até 65.535 bytes, aceita quebra de linha | `description`, `resume`, `glob` |
| Id de outra tabela maior que zero | `sessao_id`, `players_id` |
| Ficha precisa ser objeto JSON de até 1 MB | `sheet` |
| `PUT` precisa trazer ao menos um campo | todos os `PUT` |

Falhas voltam todas juntas, com status `422`:

```json
{ "error": "Dados inválidos.", "campos": [ { "campo": "name", "mensagem": "é obrigatório e não pode ser vazio" } ] }
```

Essas validações só garantem formato e o limite físico das colunas. Regras do jogo (quem pode editar o quê, limites por sessão etc.) **não** entram na API.

---

## 3. Como Adicionar um Novo CRUD de Tabela

1. Crie uma **nova** migration em `migrations/` (ex.: `0002_nova_tabela.sql`). Nunca edite uma migration já aplicada: o checksum muda e a API deixa de subir.
2. Duplique qualquer arquivo de `src/routes/` (por exemplo, copie `sessao_rpg.rs` para `src/routes/nova_tabela.rs`) e troque: a constante `TABELA` (colunas, busca e ordenação permitidas), as structs, os filtros exatos, o `validar()` e as queries.
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

A API é só um CRUD por tabela para desacoplar o cliente do banco: **não tem regra de negócio**. Valida apenas formato e limites das colunas (seção 2.2); integridade entre tabelas fica com as chaves estrangeiras do MariaDB.

| Situação | Status | Corpo |
| :--- | :---: | :--- |
| Sucesso na leitura/alteração | `200` | Registro, ou página `{items, total, limit, offset}` na listagem |
| Registro criado | `201` | Registro criado |
| Registro apagado | `204` | Vazio |
| `id` inexistente | `404` | `{"error": "..."}` |
| Campo inválido (validação) ou parâmetro de listagem inválido | `422` | `{"error": "...", "campos": [...]}` |
| JSON com tipo/campo errado | `422` | `{"error": "..."}` |
| JSON malformado, query ou id do caminho inválidos | `400` | `{"error": "..."}` |
| Sem `Content-Type: application/json` | `415` | `{"error": "..."}` |
| Corpo maior que 2 MB | `413` | `{"error": "..."}` |
| Rota inexistente | `404` | `{"error": "Rota não encontrada."}` |
| Requisição demorou mais de 30 s | `408` | Vazio |
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
