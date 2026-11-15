# RpgEmailVerifier — Verificador de Cadastro por E-mail & Site NeXTSTEP 1996

Serviço independente em Rust (`Axum` + `SQLx` + `Argon2id`) que:
1. Recebe o pedido de criação de usuário pendente vindo do aplicativo desktop **`AsyncTTRPG96`** (`POST /verify/solicitar`).
2. Exibe a página HTML no estilo **NeXTSTEP 1996 (`WebNavigator - rpgasyncst.local`)** em `GET /verify?token=ACT-98-7741-KHR`.
3. Quando o jogador abre o link do e-mail no navegador (ou confirma o código de 6 dígitos no app via `POST /verify/confirmar`), **efetiva automaticamente o cadastro do usuário no MariaDB** (cria/ativa o registro em `USUARIOS` com senha em hash Argon2id compatível com o `RpgDBCrud` e vincula o jogador na tabela `SESSAO_PLAYERS`).

---

## 1. Estrutura da Pasta `RpgEmailVerifier/`

```text
RpgEmailVerifier/
├── Cargo.toml
├── Dockerfile
├── .env.example
├── static/
│   ├── verify.html                 # Página HTML WebNavigator NeXTSTEP 1996
│   ├── logo.png                    # Emblema oficial do AsyncTTRPG96
│   └── fonts/                      # Fontes Silkscreen-Bold, Silkscreen-Regular e VT323
└── src/
    ├── main.rs                     # Servidor HTTP na porta 8081 e health-check
    └── verificador.rs              # Rotas /verify, validação de token/código e gravação no MariaDB
```

---

## 2. Rotas Disponíveis (Porta Padrão: `8081`)

| Método e Rota | Descrição |
| :--- | :--- |
| **`GET /verify?token=ACT-98-7741-KHR`** | Abre o site **WebNavigator NeXTSTEP 1996**, valida o token/código, **grava o usuário no MariaDB (`USUARIOS` + `SESSAO_PLAYERS`)** e exibe os detalhes sincronizados (`Handle / Usuário`, `Papel na Mesa`, `Sessão / Campanha`, `Servidor de Destino`, `Status da Conexão`). |
| **`POST /verify/solicitar`** | Registra um cadastro pendente no banco e retorna `{ token, codigo, verify_url, expira_em }`. |
| **`POST /verify/confirmar`** | Confirma o cadastro via JSON (usando `token` ou o `codigo` de 6 dígitos digitado na janela do app `AsyncTTRPG96`) e grava o usuário no MariaDB. |
| **`GET /verify/status?token=...`** | Consulta via JSON se o jogador já clicou no link do e-mail (`confirmado: true/false`). |
| **`GET /health`** | Health-check do serviço e da conexão com o MariaDB. |

---

## 3. Exemplo de Integração no `AsyncTTRPG96`

### 3.1. Ao clicar em `[ Criar Jogador ↵ ]` (`POST /verify/solicitar`)
Envie os dados preenchidos na tela de Criar Jogador para `http://127.0.0.1:8081/verify/solicitar`:

```json
{
  "login": "Kaelen",
  "nome": "Kaelen",
  "email": "kaelen@chronos.local",
  "senha": "minha-chave-secreta",
  "papel": "Jogador",
  "codigo_convite": "KHAROS-98",
  "sessao_nome": "Crônicas de Kharos",
  "servidor": "127.0.0.1:8080",
  "codigo": "482910"
}
```

Resposta (`201 Created`):
```json
{
  "token": "ACT-98-7741-KHR",
  "codigo": "482910",
  "verify_url": "http://127.0.0.1:8081/verify?token=ACT-98-7741-KHR",
  "login": "kaelen",
  "nome": "Kaelen",
  "email": "kaelen@chronos.local",
  "papel": "Jogador",
  "sessao_nome": "Crônicas de Kharos",
  "servidor": "127.0.0.1:8080",
  "expira_em": "2026-11-16T14:30:00Z"
}
```
> No e-mail enviado pelo `src/mail/mod.rs` do `AsyncTTRPG96`, basta incluir tanto o `codigo` de 6 dígitos quanto o link `verify_url` (`http://127.0.0.1:8081/verify?token=ACT-98-7741-KHR`).

### 3.2. Quando o Jogador Clica no Link do E-mail ou Digita o Código
- **Se clicar no link (`http://127.0.0.1:8081/verify?token=ACT-98-7741-KHR`)**:
  - O navegador abre a janela **WebNavigator - rpgasyncst.local** com **`ACESSO AUTORIZADO COM SUCESSO! ✔`** e o usuário já é inserido automaticamente na tabela `USUARIOS` e `SESSAO_PLAYERS` do MariaDB!
- **Se digitar o código de 6 dígitos dentro do aplicativo desktop (`POST /verify/confirmar`)**:
  ```json
  {
    "codigo": "482910",
    "login": "Kaelen"
  }
  ```
  O `RpgEmailVerifier` valida o código e também efetiva o cadastro no MariaDB na mesma hora.
