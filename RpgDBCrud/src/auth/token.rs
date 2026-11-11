//! Tokens aleatórios (sessão e dispositivo). O banco guarda só o SHA-256: um vazamento da
//! tabela não entrega tokens utilizáveis.

use crate::error::AppError;
use sha2::{Digest, Sha256};

/// Token de 256 bits em hexadecimal (64 caracteres).
pub fn gerar() -> Result<String, AppError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| AppError::Interno(format!("gerador aleatório: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Formato esperado de um token (evita consultar o banco com lixo).
pub fn formato_valido(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_sao_unicos_e_bem_formados() {
        let a = gerar().expect("token");
        let b = gerar().expect("token");
        assert_ne!(a, b);
        assert!(formato_valido(&a));
        assert!(!formato_valido("abc"));
        assert_eq!(hash(&a).len(), 32);
    }
}
