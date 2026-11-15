//! Extração do IP real do cliente (respeitando `TRUSTED_PROXIES`) e controle de penalidades
//! exponenciais contra força bruta na tabela `PENALIDADES`.

use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};
use ipnet::IpNet;
use sqlx::MySqlPool;
use std::net::{IpAddr, Ipv6Addr};

pub const ESPERA_INICIAL_SEGUNDOS: i64 = 10;
pub const ESPERA_MAXIMA_SEGUNDOS: i64 = 10 * 365 * 24 * 60 * 60;
pub const ESQUECER_IP_APOS_HORAS: i64 = 24;
/// Solicitações de código por IP permitidas antes de a espera exponencial começar.
pub const SOLICITACOES_LIVRES_POR_IP: u32 = 3;

/// IP real do cliente. Cabeçalhos `X-Forwarded-For` e `X-Real-IP` só são aceitos quando
/// a conexão vem de um proxy confiável (`TRUSTED_PROXIES`).
pub fn ip_cliente(par: IpAddr, headers: &HeaderMap, confiaveis: &[IpNet]) -> IpAddr {
    let par = normalizar(par);
    let confiavel = |ip: &IpAddr| confiaveis.iter().any(|rede| rede.contains(ip));
    if !confiavel(&par) {
        return par;
    }

    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        for parte in xff.rsplit(',') {
            match parte.trim().parse::<IpAddr>().map(normalizar) {
                Ok(ip) if confiavel(&ip) => continue,
                Ok(ip) => return ip,
                Err(_) => break,
            }
        }
    }

    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<IpAddr>().ok())
        .map(normalizar)
        .unwrap_or(par)
}

/// Chave de penalidade do IP (IPv6 agrupado por `/64`).
pub fn chave_ip(prefixo: &str, ip: IpAddr) -> String {
    match normalizar(ip) {
        IpAddr::V4(v4) => format!("{prefixo}:{v4}"),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            let rede = Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0);
            format!("{prefixo}:{rede}/64")
        }
    }
}

pub fn chave_conta_verif(login: &str) -> String {
    format!("verif_conta:{login}")
}

fn normalizar(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// Lê a lista de proxies confiáveis (`TRUSTED_PROXIES`, separada por vírgula).
pub fn ler_proxies(texto: &str) -> Result<Vec<IpNet>, String> {
    texto
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<IpNet>()
                .or_else(|_| s.parse::<IpAddr>().map(IpNet::from))
                .map_err(|_| format!("TRUSTED_PROXIES: endereço inválido '{s}'"))
        })
        .collect()
}

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

pub fn segundos_ate(ate: DateTime<Utc>) -> i64 {
    let ms = (ate - Utc::now()).num_milliseconds().max(0);
    (ms + 999) / 1000
}

pub async fn bloqueio(pool: &MySqlPool, chave: &str) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let linha: Option<(Option<DateTime<Utc>>,)> =
        sqlx::query_as("SELECT bloqueado_ate FROM PENALIDADES WHERE chave = ?")
            .bind(chave)
            .fetch_optional(pool)
            .await?;
    let agora = Utc::now();
    Ok(linha.and_then(|(ate,)| ate).filter(|ate| *ate > agora))
}

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

pub async fn registrar(
    pool: &MySqlPool,
    chave: &str,
    livres: u32,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let agora = Utc::now();
    let mut tx = pool.begin().await?;

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

    let expirou =
        bloqueado_ate.is_some_and(|ate| agora - ate > Duration::hours(ESQUECER_IP_APOS_HORAS));
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
    Ok(novo_bloqueio)
}

pub async fn limpar(pool: &MySqlPool, chave: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM PENALIDADES WHERE chave = ?")
        .bind(chave)
        .execute(pool)
        .await?
        .rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn espera_exponencial_dobra_a_cada_falha() {
        assert_eq!(espera_para(0).num_seconds(), 0);
        assert_eq!(espera_para(1).num_seconds(), 10);
        assert_eq!(espera_para(2).num_seconds(), 20);
        assert_eq!(espera_para(3).num_seconds(), 40);
        assert_eq!(espera_para(u32::MAX).num_seconds(), ESPERA_MAXIMA_SEGUNDOS);
    }

    #[test]
    fn extrai_ip_real_apenas_de_proxy_confiavel() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.7, 172.18.0.2"),
        );
        let par: IpAddr = "172.18.0.2".parse().expect("ip");
        let sem_proxy = ip_cliente(par, &h, &[]);
        assert_eq!(sem_proxy, par);

        let proxies = ler_proxies("172.18.0.0/16").expect("cidr");
        let com_proxy = ip_cliente(par, &h, &proxies);
        assert_eq!(com_proxy, "198.51.100.7".parse::<IpAddr>().expect("ip"));
    }
}
