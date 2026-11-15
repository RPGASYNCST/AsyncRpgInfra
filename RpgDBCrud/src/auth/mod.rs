//! Autenticação: cadastro, login com token de sessão, dispositivos confiáveis e
//! penalidade exponencial contra força bruta.
//!
//! Como a penalidade funciona (ver `penalidade.rs`):
//! - Cada login errado dobra a espera: 10 s, 20 s, 40 s, 1 min 20 s, 2 min 40 s...
//! - Durante a espera, tentativas são recusadas (429) sem conferir a senha e sem contar.
//! - Login de **dispositivo confiável** (token entregue após um login correto naquele
//!   computador) só sofre a penalidade do próprio dispositivo: o atacante errando a senha
//!   não bloqueia o dono da conta no computador de sempre.
//! - Login de dispositivo desconhecido sofre a penalidade da **conta** e do **IP** ao mesmo
//!   tempo; trocar de IP não ajuda o atacante, porque a conta continua acumulando.
//! - Acerto zera a conta e o dispositivo. A do IP só expira 24 h após o fim do bloqueio
//!   (com cadastro aberto, o atacante não pode "limpar" o IP entrando na própria conta).
//! - Um admin pode zerar a penalidade de uma conta ou de um IP.

pub mod admin;
pub mod ip;
pub mod middleware;
pub mod penalidade;
pub mod rotas;
pub mod senha;
pub mod token;

use crate::error::AppError;
use sqlx::MySqlPool;

/// Cria (ou promove a admin) o usuário informado em `ADMIN_LOGIN`/`ADMIN_SENHA`.
/// Se o usuário já existe, a senha NÃO é alterada; só garante o perfil de admin.
pub async fn garantir_admin_inicial(
    pool: &MySqlPool,
    login: &str,
    senha: &str,
) -> Result<(), AppError> {
    let login = login.trim().to_lowercase();
    let existente: Option<(u64,)> = sqlx::query_as("SELECT id FROM USUARIOS WHERE login = ?")
        .bind(&login)
        .fetch_optional(pool)
        .await?;

    if let Some((id,)) = existente {
        sqlx::query("UPDATE USUARIOS SET admin = TRUE, ativo = TRUE WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        tracing::info!("Usuário admin inicial '{login}' já existia; perfil de admin garantido.");
        return Ok(());
    }

    let mut v = crate::validacao::Validador::new();
    rotas::validar_login(&mut v, &login);
    rotas::validar_senha(&mut v, senha, &login);
    v.finalizar()?;

    let hash = senha::gerar_hash(senha.to_string()).await?;
    sqlx::query("INSERT INTO USUARIOS (login, nome, senha_hash, admin) VALUES (?, ?, ?, TRUE)")
        .bind(&login)
        .bind("Administrador")
        .bind(hash)
        .execute(pool)
        .await?;
    tracing::info!("Usuário admin inicial '{login}' criado.");
    Ok(())
}

/// Limpeza periódica: sessões vencidas, penalidades expiradas e códigos de verificação vencidos.
pub async fn limpar_expirados(pool: &MySqlPool) -> Result<(), sqlx::Error> {
    let agora = chrono::Utc::now();
    let sessoes = sqlx::query("DELETE FROM SESSOES_AUTH WHERE expira_em < ?")
        .bind(agora)
        .execute(pool)
        .await?
        .rows_affected();
    let penalidades = penalidade::limpar_expiradas(pool, agora).await?;
    let verificacoes =
        sqlx::query("DELETE FROM VERIFICACOES_CADASTRO WHERE confirmado = FALSE AND expira_em < ?")
            .bind(agora)
            .execute(pool)
            .await
            .map(|r| r.rows_affected())
            .unwrap_or(0);
    if sessoes > 0 || penalidades > 0 || verificacoes > 0 {
        tracing::info!(
            "Limpeza: {sessoes} sessão(ões) vencida(s), {penalidades} penalidade(s) expirada(s) e {verificacoes} verificação(ões) vencida(s) removidas."
        );
    }
    Ok(())
}
