//! Envio de e-mails de verificação via SMTP (`lettre`).

use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::env;

#[derive(Debug, Clone)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from_email: String,
    pub from_name: String,
    pub use_tls: bool,
}

impl SmtpSettings {
    pub fn from_env() -> Option<Self> {
        let host = env::var("SMTP_HOST")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let port = env::var("SMTP_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(587);
        let username = env::var("SMTP_USERNAME").unwrap_or_default();
        let password = env::var("SMTP_PASSWORD").unwrap_or_default();
        let from_email = env::var("SMTP_FROM_EMAIL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| username.clone());
        let from_name = env::var("SMTP_FROM_NAME")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "RPGASYNCST".to_string());
        let use_tls = env::var("SMTP_USE_TLS")
            .ok()
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0" | "no"))
            .unwrap_or(true);

        if from_email.is_empty() {
            return None;
        }

        Some(Self {
            host,
            port,
            username,
            password,
            from_email,
            from_name,
            use_tls,
        })
    }
}

pub async fn enviar_email_verificacao(
    smtp: &SmtpSettings,
    to_email: &str,
    handle: &str,
    sessao_nome: &str,
    servidor: &str,
    codigo: &str,
    token: &str,
    verify_url: &str,
) -> Result<(), String> {
    let from_mailbox: Mailbox = format!("{} <{}>", smtp.from_name, smtp.from_email)
        .parse()
        .map_err(|e| format!("Endereço remetente inválido: {e}"))?;

    let to_mailbox: Mailbox = format!("{handle} <{to_email}>")
        .parse()
        .or_else(|_| to_email.parse())
        .map_err(|e| format!("Endereço de destino inválido: {e}"))?;

    let body = format!(
        "Saudações, {handle}!\n\n\
         Seu cadastro de operador/jogador no RPGASYNCST foi iniciado!\n\
         • Sessão / Campanha: {sessao_nome}\n\
         • Servidor de Destino: {servidor}\n\n\
         ========================================================\n\
         1. CONFIRMAR PELO NAVEGADOR (RECOMENDADO):\n\
            Clique no link abaixo para abrir o WebNavigator e ativar sua conta automaticamente:\n\
            {verify_url}\n\
            (Token: {token})\n\n\
         2. OU CONFIRMAR PELO APLICATIVO DESKTOP:\n\
            Código de 6 dígitos: {codigo}\n\
         ========================================================\n\n\
         Se você não solicitou este cadastro, ignore este e-mail.\n\n\
         -- RPGASYNCST Network Daemon v1.4 • NeXTSTEP Edition • © 1996"
    );

    let email = Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(format!(
            "[RPGASYNCST] Confirmação de Cadastro — Código: {codigo} ({token})"
        ))
        .body(body)
        .map_err(|e| format!("Erro ao montar e-mail: {e}"))?;

    let mut builder = if smtp.use_tls {
        if smtp.port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host)
                .map_err(|e| format!("Erro TLS SMTP: {e}"))?
                .port(smtp.port)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
                .map_err(|e| format!("Erro STARTTLS SMTP: {e}"))?
                .port(smtp.port)
        }
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host).port(smtp.port)
    };

    if !smtp.username.trim().is_empty() {
        builder = builder.credentials(Credentials::new(
            smtp.username.clone(),
            smtp.password.clone(),
        ));
    }

    let mailer = builder.build();
    mailer
        .send(email)
        .await
        .map_err(|e| format!("Falha ao enviar e-mail via SMTP: {e}"))?;

    tracing::info!(
        "E-mail de verificação enviado via SMTP para '{to_email}' (usuário '{handle}')."
    );
    Ok(())
}
