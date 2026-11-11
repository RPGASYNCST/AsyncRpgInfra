# AsyncRpgInfra — Pipeline Centralizada de CI & Banco MariaDB (Rust, Iced & SQLx)

Repositório central de infraestrutura de Integração Contínua (CI), qualidade de código, auditoria de segurança e **deploy do banco de dados MariaDB** da organização **`RPGASYNCST`** para aplicações desenvolvidas em **Rust** utilizando **Iced**, **SQLx** e múltiplas dependências.

Este repositório opera em **Runner Próprio (Self-Hosted Linux)** **sem uso de `sudo`** durante a execução dos jobs.

---

## 1. Fluxo Completo da Arquitetura

```mermaid
flowchart TD
    Dev["Desenvolvedor cria branch e abre Pull Request (MR)<br/>para develop ou main no repo da Aplicação"] --> Trigger["GitHub Actions do repo da Aplicação dispara"]
    Trigger --> Call["Chama RPGASYNCST/AsyncRpgInfra/.github/workflows/ci.yml@main"]
    Call --> Runner["Self-Hosted Runner (Linux)"]

    subgraph Pipeline ["Execução da CI no Runner Próprio (Sem Sudo)"]
        direction TB
        J1["1. Formatação & Deps<br/>(cargo fmt + cargo machete)"]
        J2["2. Segurança & Licenças<br/>(cargo audit + cargo deny)"]
        J3["3. Check & Clippy + MariaDB CI (Porta 3307 RAM)<br/>(sqlx migrate run + cargo check + clippy)"]
        J4["4. Testes Automatizados<br/>(xvfb-run cargo nextest + #[sqlx::test] + doctests)"]
        J5["5. Documentação & Cobertura<br/>(cargo doc + cargo llvm-cov)"]
        Gate["6. CI Gate<br/>(Remove MariaDB CI + Valida todos os jobs)"]

        J3 --> J4
        J3 --> J5
        J1 --> Gate
        J2 --> Gate
        J3 --> Gate
        J4 --> Gate
        J5 --> Gate
    end

    Runner --> Pipeline
    Gate -->|Sucesso| Merge["Ruleset libera o botão de Merge no PR"]
    Gate -->|Falha| Block["Ruleset bloqueia o Merge na main / develop"]
    Merge -->|Merge na main / deploy| DeployDB["deploy-mariadb.yml<br/>Sobe MariaDB (Porta 3306) + API REST RpgDBCrud (Porta 8080)"]
    Exe["Aplicação Cliente (.exe Iced)"] <-->|HTTP JSON :8080| DeployDB
```

---

## 2. Estrutura deste Repositório

| Arquivo / Pasta | Função |
| :--- | :--- |
| [`RpgDBCrud/`](RpgDBCrud/README.md) | **API REST CRUD** em Rust (`Axum` + `SQLx`) na porta `8080` para que a aplicação `.exe` consulte as tabelas (`SESSAO_RPG`, `SESSAO_ASSETS`, `SESSAO_PLAYERS`, `PLAYERS_SHEETS`) via HTTP/JSON sem acessar o MariaDB diretamente. |
| [`.github/workflows/ci.yml`](.github/workflows/ci.yml) | Workflow reutilizável (`workflow_call`) com os jobs de validação, banco MariaDB efêmero para `sqlx` (`porta 3307`), testes, cobertura e `CI Gate`. |
| [`.github/workflows/deploy-mariadb.yml`](.github/workflows/deploy-mariadb.yml) | Workflow de deploy/atualização do container **MariaDB 11.4 LTS** da aplicação (`porta 3306`, com volume persistente) e da API `RpgDBCrud` (`porta 8080`), que aplica as próprias migrations ao subir. |
| [`database/docker-compose.yml`](database/docker-compose.yml) | Definição Docker Compose do MariaDB da aplicação com volume persistente (`async_rpg_mariadb_data`), rede isolada e `healthcheck` nativo. |
| [`database/conf.d/custom.cnf`](database/conf.d/custom.cnf) | Configuração do MariaDB otimizada para Rust/`sqlx` (`utf8mb4`, `utf8mb4_unicode_ci`, `UTC`, InnoDB tuning). |
| [`database/init/01-init.sql`](database/init/01-init.sql) | Script SQL executado automaticamente na criação inicial do volume do MariaDB. |
| [`database/.env.example`](database/.env.example) | Modelo de variáveis de ambiente e `DATABASE_URL` para conexão com o MariaDB. |
| [`rustfmt.toml`](rustfmt.toml) | Regras padronizadas de formatação (`edition = "2021"`, `max_width = 100`, reordenação de imports). |
| [`clippy.toml`](clippy.toml) | Limites de complexidade cognitiva e permissão de `unwrap`/`expect`/`dbg!` apenas em testes. |
| [`deny.toml`](deny.toml) | Regras do `cargo-deny`: auditoria de advisories, bloqueio de wildcards e licenças permitidas pela árvore do **Iced / wgpu / sqlx**. |
| [`.config/nextest.toml`](.config/nextest.toml) | Perfil `ci` do `cargo-nextest` com *retries*, timeouts e exportação de relatório JUnit (`junit.xml`). |
| [`.cargo/config.toml`](.cargo/config.toml) | Configura o linker **`lld`** via `clang` no Linux e atalhos locais do Cargo. |

---

## 3. Como Funciona o MariaDB (Aplicação vs. CI)

Para garantir que os testes da pipeline **nunca** sujem ou apaguem dados reais da aplicação no mesmo servidor Linux, a arquitetura separa o MariaDB em **duas instâncias independentes**:

| Instância | Container Docker | Porta | Armazenamento | Quando Roda |
| :--- | :--- | :--- | :--- | :--- |
| **MariaDB da Aplicação (Persistente)** | `async-rpg-mariadb` | `3306` *(configurável)* | Volume Docker persistente (`async_rpg_mariadb_data`) em `$HOME/async-rpg-mariadb` | Sempre ativo (`restart: unless-stopped`). Atualizado pelo workflow [`deploy-mariadb.yml`](.github/workflows/deploy-mariadb.yml). |
| **MariaDB de Testes da CI (Efêmero)** | `async-rpg-mariadb-ci` | `3307` (`127.0.0.1`) | 100% em memória RAM (`--tmpfs /var/lib/mysql`) | Criado no início do job `lint-and-check`, usado pelo `sqlx` (check, clippy e `#[sqlx::test]`) e destruído automaticamente no job `ci-gate`. |

### Suporte Automático ao `sqlx` na CI
- A variável `DATABASE_URL="mysql://rpg_ci:rpg_ci_pass@127.0.0.1:3307/async_rpg_ci"` já fica exportada automaticamente em todos os jobs da CI.
- Se o repositório da aplicação tiver uma pasta **`migrations/`**, a pipeline instala o `sqlx-cli` e executa `sqlx migrate run` antes do `cargo check` e `cargo clippy` (permitindo que as macros `sqlx::query!` validem o SQL em tempo de compilação contra o schema real do MariaDB).
- O usuário de teste `rpg_ci` recebe permissão `GRANT ALL PRIVILEGES ON *.*` no container efêmero para que testes anotados com `#[sqlx::test]` possam criar e destruir bancos isolados livremente.

---

## 4. Preparação do Servidor Linux (Self-Hosted Runner)

Como a pipeline roda **100% sem `sudo`**, o administrador deve preparar a máquina Linux **uma única vez**:

### 4.1. Instalar pacotes nativos do Iced, compilação e Docker
```bash
sudo apt-get update && sudo apt-get install -y --no-install-recommends \
  curl git ca-certificates tar jq \
  docker.io docker-compose-v2 \
  build-essential pkg-config cmake clang lld \
  libssl-dev libudev-dev libasound2-dev libexpat1-dev \
  libfreetype6-dev libfontconfig1-dev libxcb1-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev \
  libinerama-dev libgl1-mesa-dev libvulkan-dev xvfb
```

### 4.2. Permitir que o usuário do Runner use Docker sem `sudo`
Adicione o usuário que roda o runner ao grupo `docker`:
```bash
sudo usermod -aG docker <usuario-do-runner>
```
*(Se o serviço do runner já estiver rodando, reinicie-o com `sudo systemctl restart actions.runner.*` para aplicar o novo grupo).*

### 4.3. Instalar o Rust no usuário do Runner (sem `sudo`)
Logado no usuário Linux que executará o serviço do runner:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --component rustfmt,clippy,llvm-tools-preview
source "$HOME/.cargo/env"
```

---

## 5. Configuração do Deploy do MariaDB da Aplicação

### 5.1. Cadastrar as Secrets do Banco no GitHub
No repositório `RPGASYNCST/AsyncRpgInfra` (ou nos Secrets da organização `RPGASYNCST`), vá em **Settings → Secrets and variables → Actions** e crie as seguintes **Repository/Organization Secrets**:

| Secret | Obrigatória? | Descrição / Valor Padrão |
| :--- | :---: | :--- |
| `MARIADB_ROOT_PASSWORD` | **Sim** | Senha do usuário `root` do MariaDB. |
| `MARIADB_PASSWORD` | **Sim** | Senha do usuário da aplicação. |
| `MARIADB_USER` | Opcional | Nome do usuário da aplicação (padrão: `async_rpg_user`). |
| `MARIADB_DATABASE` | Opcional | Nome do banco de dados (padrão: `async_rpg`). |
| `MARIADB_PORT` | Opcional | Porta exposta no host (padrão: `3306`). |

### 5.2. Como Disparar o Deploy do MariaDB
- **Automaticamente**: Qualquer alteração na pasta `database/` mergeada na branch `main` do `AsyncRpgInfra` dispara o workflow [`deploy-mariadb.yml`](.github/workflows/deploy-mariadb.yml).
- **Manualmente**: Na aba **Actions** do GitHub → selecione **Deploy MariaDB (Aplicação)** → clique em **Run workflow**.
- **Pelo Repositório da Aplicação**: O repositório da aplicação também pode chamar o `deploy-mariadb.yml` após um merge na `main` para garantir que o banco e a API estão no ar. O schema é responsabilidade exclusiva das migrations da API (`RpgDBCrud/migrations`); o deploy não roda migrations de outros repositórios.

---

## 6. Como Conectar o Repositório da Aplicação

### 6.1. Liberar o acesso no `AsyncRpgInfra`
No repositório `RPGASYNCST/AsyncRpgInfra` no GitHub:
1. Vá em **Settings → Actions → General**.
2. Na seção **Access**, marque **Accessible from repositories in the 'RPGASYNCST' organization** e clique em **Save**.

### 6.2. Adicionar o `.github/workflows/ci.yml` no repositório da Aplicação
No repositório onde está o código Rust da sua aplicação (com o `Cargo.lock` commitado no Git), crie o arquivo **`.github/workflows/ci.yml`**:

```yaml
name: CI & Database

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main, develop]
  workflow_dispatch:

jobs:
  # 1. Roda todos os testes (fmt, machete, audit, deny, clippy, nextest + MariaDB de teste, docs, coverage)
  pipeline:
    uses: RPGASYNCST/AsyncRpgInfra/.github/workflows/ci.yml@main
    secrets: inherit

  # 2. (Opcional) Quando código é mergeado na main, garante o MariaDB e a API de produção ativos
  deploy-database:
    if: github.event_name == 'push' && github.ref == 'refs/heads/main'
    needs: [pipeline]
    uses: RPGASYNCST/AsyncRpgInfra/.github/workflows/deploy-mariadb.yml@main
    secrets: inherit
```

---

## 7. Como Bloquear Merge Direto na `main` e `develop` (Rulesets)

1. Acesse o repositório da **Aplicação** no GitHub e vá em **Settings → Rules → Rulesets**.
2. Clique em **New ruleset → New branch ruleset**.
3. Configure:
   - **Ruleset Name**: `Protecao main e develop`
   - **Enforcement status**: **Active**
   - **Bypass list**: Vazia.
4. Em **Target branches**:
   - Adicione a **Default branch** (`main`) e o pattern **`develop`**.
5. Em **Branch rules**, marque:
   - **[x] Restrict deletions**
   - **[x] Require a pull request before merging**
   - **[x] Require status checks to pass** → clique em **+ Add checks** e adicione:
     - **`pipeline / CI Gate (Todos os Testes Passaram)`**
   - **[x] Block force pushes**
6. Clique em **Create**.
