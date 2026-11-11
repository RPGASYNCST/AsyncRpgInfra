//! Descobre o IP real do cliente e gera a chave de penalidade dele.

use axum::http::HeaderMap;
use ipnet::IpNet;
use std::net::{IpAddr, Ipv6Addr};

/// IP real do cliente. Cabeçalhos `X-Forwarded-For`/`X-Real-IP` só são aceitos quando a
/// conexão vem de um proxy confiável; caso contrário, qualquer um poderia forjar o IP e
/// escapar da penalidade.
pub fn ip_cliente(par: IpAddr, headers: &HeaderMap, confiaveis: &[IpNet]) -> IpAddr {
    let par = normalizar(par);
    let confiavel = |ip: &IpAddr| confiaveis.iter().any(|rede| rede.contains(ip));
    if !confiavel(&par) {
        return par;
    }

    // X-Forwarded-For: "cliente, proxy1, proxy2". Lê da direita para a esquerda e para no
    // primeiro endereço que não é um proxy confiável.
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

/// Chave de penalidade do IP. IPv6 é agrupado por /64, porque um único cliente costuma
/// controlar a faixa inteira e poderia trocar de endereço a cada tentativa.
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

/// Converte IPv4 mapeado em IPv6 (`::ffff:1.2.3.4`) para IPv4.
fn normalizar(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// Lê a lista de proxies confiáveis (`TRUSTED_PROXIES`, separada por vírgula). Aceita IPs
/// e redes CIDR, ex.: `172.18.0.0/16, 10.0.0.5`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("ip válido")
    }

    fn headers(xff: Option<&str>, real: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(v) = xff {
            h.insert("x-forwarded-for", HeaderValue::from_str(v).expect("header"));
        }
        if let Some(v) = real {
            h.insert("x-real-ip", HeaderValue::from_str(v).expect("header"));
        }
        h
    }

    #[test]
    fn sem_proxy_confiavel_ignora_cabecalhos_forjados() {
        let h = headers(Some("1.1.1.1"), Some("2.2.2.2"));
        assert_eq!(ip_cliente(ip("203.0.113.9"), &h, &[]), ip("203.0.113.9"));
    }

    #[test]
    fn atras_do_proxy_usa_ultimo_ip_nao_confiavel() {
        let proxies = ler_proxies("172.18.0.0/16").expect("proxies");
        // O cliente tentou forjar "6.6.6.6"; o proxy acrescentou o IP real 198.51.100.7.
        let h = headers(Some("6.6.6.6, 198.51.100.7"), None);
        assert_eq!(
            ip_cliente(ip("172.18.0.2"), &h, &proxies),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn atras_do_proxy_sem_xff_usa_x_real_ip() {
        let proxies = ler_proxies("10.0.0.5").expect("proxies");
        let h = headers(None, Some("198.51.100.8"));
        assert_eq!(ip_cliente(ip("10.0.0.5"), &h, &proxies), ip("198.51.100.8"));
    }

    #[test]
    fn ipv6_agrupado_por_64_e_ipv4_mapeado_normalizado() {
        assert_eq!(
            chave_ip("ip", ip("2001:db8:1:2:aaaa::1")),
            chave_ip("ip", ip("2001:db8:1:2:bbbb::9"))
        );
        assert_eq!(chave_ip("ip", ip("::ffff:192.0.2.1")), "ip:192.0.2.1");
    }

    #[test]
    fn proxies_invalidos_sao_rejeitados() {
        assert!(ler_proxies("abc").is_err());
        assert!(ler_proxies("").expect("vazio").is_empty());
    }
}
