//! Hash de senha com Argon2id (parâmetros padrão recomendados pela OWASP: 19 MiB, t=2, p=1).
//! O cálculo é pesado de propósito, por isso roda fora do executor assíncrono.

use crate::error::AppError;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use std::sync::OnceLock;

/// Hash de uma senha aleatória, usado quando o login não existe: a verificação leva o mesmo
/// tempo e o atacante não descobre quais logins existem medindo a resposta.
static HASH_FICTICIO: OnceLock<String> = OnceLock::new();

fn calcular_hash(senha: &str) -> Result<String, AppError> {
    let mut sal = [0u8; 16];
    getrandom::fill(&mut sal).map_err(|e| AppError::Interno(format!("gerador aleatório: {e}")))?;
    let sal = SaltString::encode_b64(&sal).map_err(|e| AppError::Interno(format!("sal: {e}")))?;
    Argon2::default()
        .hash_password(senha.as_bytes(), &sal)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Interno(format!("argon2: {e}")))
}

pub async fn gerar_hash(senha: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || calcular_hash(&senha))
        .await
        .map_err(|e| AppError::Interno(format!("tarefa de hash: {e}")))?
}

/// Confere a senha. Com `hash = None` (login inexistente) compara com um hash fictício e
/// devolve `false`, gastando o mesmo tempo de uma verificação real.
pub async fn verificar(senha: String, hash: Option<String>) -> Result<bool, AppError> {
    tokio::task::spawn_blocking(move || {
        let existe = hash.is_some();
        let hash = match hash {
            Some(h) => h,
            None => match HASH_FICTICIO.get() {
                Some(h) => h.clone(),
                None => {
                    let h = calcular_hash("senha-ficticia-para-tempo-constante")?;
                    HASH_FICTICIO.get_or_init(|| h).clone()
                }
            },
        };
        let parsed = PasswordHash::new(&hash)
            .map_err(|e| AppError::Interno(format!("hash inválido: {e}")))?;
        let confere = Argon2::default()
            .verify_password(senha.as_bytes(), &parsed)
            .is_ok();
        Ok(existe && confere)
    })
    .await
    .map_err(|e| AppError::Interno(format!("tarefa de verificação: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_confere_so_com_a_senha_certa() {
        let hash = gerar_hash("senha-muito-secreta".into())
            .await
            .expect("hash");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verificar("senha-muito-secreta".into(), Some(hash.clone()))
            .await
            .expect("verificar"));
        assert!(!verificar("outra-senha".into(), Some(hash))
            .await
            .expect("verificar"));
    }

    #[tokio::test]
    async fn login_inexistente_sempre_falha() {
        assert!(!verificar("qualquer".into(), None).await.expect("verificar"));
    }
}
