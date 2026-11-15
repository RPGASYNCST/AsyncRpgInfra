//! Penalidade exponencial por chave (`ip:`, `conta:`, `disp:`, `cadastro:`).
//!
//! Depois da n-ésima falha, a chave fica bloqueada por `10 s * 2^(n-1)`: 10 s, 20 s, 40 s,
//! 1 min 20 s, 2 min 40 s... Na 20ª falha a espera passa de 60 dias, o que na prática
//! inviabiliza força bruta.
//!
//! Esquecimento:
//! - `conta:` e `disp:` só zeram com login correto ou ação de um admin.
//! - `ip:` e `cadastro:` zeram sozinhas 24 h após o fim do último bloqueio, porque IPs mudam
//!   de dono (operadoras reaproveitam endereços) e não queremos punir um inocente para sempre.

use chrono::{DateTime, Duration, Utc};
use sqlx::MySqlPool;

pub const ESPERA_INICIAL_SEGUNDOS: i64 = 10;
/// Teto da espera (10 anos): evita estouro de data, mas na prática é bloqueio permanente.
pub const ESPERA_MAXIMA_SEGUNDOS: i64 = 10 * 365 * 24 * 60 * 60;
/// Tempo, após o fim do bloqueio, para chaves de IP e cadastro serem esquecidas.
pub const ESQUECER_IP_APOS_HORAS: i64 = 24;

/// Espera aplicada após a falha número `falhas` (1 = primeira falha).
pub fn espera_para(falhas: u32) -> Duration {
    if falhas == 0 {
        return Duration::zero();
    }
    let expoente = (falhas - 1).min(40);
    let segundos = ESPERA_INICIAL_SEGUNDOS
        .saturating_mul(1_i64 << expoente)
        .min(ESPERA_MAXIMA_SEGUNDOS);
    Duration::seconds(segundos)
}

fn esquece_sozinha(chave: &str) -> bool {
    chave.starts_with("ip:") || chave.starts_with("cadastro:")
}

/// Momento até quando a chave está bloqueada, se ainda estiver.
pub async fn bloqueio(pool: &MySqlPool, chave: &str) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let linha: Option<(Option<DateTime<Utc>>,)> =
        sqlx::query_as("SELECT bloqueado_ate FROM PENALIDADES WHERE chave = ?")
            .bind(chave)
            .fetch_optional(pool)
            .await?;
    let agora = Utc::now();
    Ok(linha.and_then(|(ate,)| ate).filter(|ate| *ate > agora))
}

/// Maior bloqueio ativo entre as chaves informadas.
pub async fn maior_bloqueio(
    pool: &MySqlPool,
    chaves: &[String],
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let mut maior = None;
    for chave in chaves {
        if let Some(ate) = bloqueio(pool, chave).await? {
            maior = Some(maior.map_or(ate, |m: DateTime<Utc>| m.max(ate)));
        }
    }
    Ok(maior)
}

/// Registra um evento (falha de login ou cadastro) e devolve até quando a chave ficou
/// bloqueada. `livres` = quantos eventos são permitidos antes de a espera começar
/// (0 no login: a primeira falha já espera 10 s).
pub async fn registrar(
    pool: &MySqlPool,
    chave: &str,
    livres: u32,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let agora = Utc::now();
    let mut tx = pool.begin().await?;

    // Garante a linha e trava para evitar corrida entre tentativas simultâneas.
    sqlx::query(
        "INSERT IGNORE INTO PENALIDADES (chave, falhas, bloqueado_ate, ultima_falha_em) \
         VALUES (?, 0, NULL, ?)",
    )
    .bind(chave)
    .bind(agora)
    .execute(&mut *tx)
    .await?;
    let (falhas, bloqueado_ate): (u32, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT falhas, bloqueado_ate FROM PENALIDADES WHERE chave = ? FOR UPDATE")
            .bind(chave)
            .fetch_one(&mut *tx)
            .await?;

    let expirou = esquece_sozinha(chave)
        && bloqueado_ate.is_some_and(|ate| agora - ate > Duration::hours(ESQUECER_IP_APOS_HORAS));
    let falhas = if expirou { 1 } else { falhas.saturating_add(1) };

    let espera = espera_para(falhas.saturating_sub(livres));
    let novo_bloqueio = (espera > Duration::zero()).then(|| agora + espera);

    sqlx::query(
        "UPDATE PENALIDADES SET falhas = ?, bloqueado_ate = ?, ultima_falha_em = ? WHERE chave = ?",
    )
    .bind(falhas)
    .bind(novo_bloqueio)
    .bind(agora)
    .bind(chave)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    if falhas >= 5 {
        tracing::warn!("Penalidade '{chave}': {falhas} falhas; bloqueado até {novo_bloqueio:?}");
    }
    Ok(novo_bloqueio)
}

pub async fn limpar(pool: &MySqlPool, chave: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM PENALIDADES WHERE chave = ?")
        .bind(chave)
        .execute(pool)
        .await?
        .rows_affected())
}

/// Remove a penalidade da conta, de todos os dispositivos do usuário e de verificação.
pub async fn limpar_usuario(
    pool: &MySqlPool,
    usuario_id: u64,
    login: &str,
) -> Result<u64, sqlx::Error> {
    let conta = limpar(pool, &chave_conta(login)).await?;
    let verif = limpar(pool, &format!("verif_conta:{login}")).await?;
    let dispositivos = sqlx::query(
        "DELETE FROM PENALIDADES WHERE chave IN \
         (SELECT CONCAT('disp:', id) FROM DISPOSITIVOS WHERE usuario_id = ?)",
    )
    .bind(usuario_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(conta + verif + dispositivos)
}

/// Apaga penalidades que não têm mais efeito: IP/cadastro esquecidos e contas inexistentes
/// (tentativas contra logins que nem existem) sem bloqueio ativo há 24 h.
pub async fn limpar_expiradas(pool: &MySqlPool, agora: DateTime<Utc>) -> Result<u64, sqlx::Error> {
    let limite = agora - Duration::hours(ESQUECER_IP_APOS_HORAS);
    let ip = sqlx::query(
        "DELETE FROM PENALIDADES WHERE (chave LIKE 'ip:%' OR chave LIKE 'cadastro:%') \
         AND ultima_falha_em < ? AND (bloqueado_ate IS NULL OR bloqueado_ate < ?)",
    )
    .bind(limite)
    .bind(limite)
    .execute(pool)
    .await?
    .rows_affected();
    let contas_inexistentes = sqlx::query(
        "DELETE FROM PENALIDADES WHERE (chave LIKE 'conta:%' OR chave LIKE 'verif_conta:%') \
         AND SUBSTRING_INDEX(chave, ':', -1) NOT IN (SELECT login FROM USUARIOS) \
         AND ultima_falha_em < ? AND (bloqueado_ate IS NULL OR bloqueado_ate < ?)",
    )
    .bind(limite)
    .bind(limite)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(ip + contas_inexistentes)
}

pub fn chave_conta(login: &str) -> String {
    format!("conta:{login}")
}

pub fn chave_dispositivo(id: u64) -> String {
    format!("disp:{id}")
}

pub fn segundos_ate(ate: DateTime<Utc>) -> i64 {
    // Arredonda para cima: "aguarde 0 s" confundiria o usuário.
    let ms = (ate - Utc::now()).num_milliseconds().max(0);
    (ms + 999) / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn espera_dobra_a_partir_de_10_segundos() {
        let esperado = [0, 10, 20, 40, 80, 160, 320];
        for (falhas, segundos) in esperado.iter().enumerate() {
            assert_eq!(
                espera_para(falhas as u32).num_seconds(),
                *segundos,
                "falhas={falhas}"
            );
        }
    }

    #[test]
    fn vigesima_falha_passa_de_60_dias_e_teto_nao_estoura() {
        assert!(espera_para(20).num_seconds() > 60 * 24 * 60 * 60);
        assert_eq!(espera_para(500).num_seconds(), ESPERA_MAXIMA_SEGUNDOS);
        assert_eq!(espera_para(u32::MAX).num_seconds(), ESPERA_MAXIMA_SEGUNDOS);
    }

    #[test]
    fn so_ip_e_cadastro_esquecem_sozinhos() {
        assert!(esquece_sozinha("ip:1.2.3.4"));
        assert!(esquece_sozinha("cadastro:1.2.3.4"));
        assert!(!esquece_sozinha("conta:ana"));
        assert!(!esquece_sozinha("disp:7"));
    }
}
