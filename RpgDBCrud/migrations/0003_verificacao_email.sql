-- Verificação de criação de usuário por e-mail (Token de link web e código de 6 dígitos)

ALTER TABLE USUARIOS
    ADD COLUMN IF NOT EXISTS email VARCHAR(255) NULL AFTER nome,
    ADD COLUMN IF NOT EXISTS email_verificado BOOLEAN NOT NULL DEFAULT FALSE AFTER ativo;

CREATE TABLE IF NOT EXISTS VERIFICACOES_CADASTRO (
    id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    -- Token único para o link de verificação (ex: ACT-98-7741-KHR)
    token VARCHAR(64) NOT NULL,
    -- Código numérico de 6 dígitos (ex: 482910)
    codigo VARCHAR(16) NOT NULL,
    -- Dados do Mestre (Host) e da Sessão a serem gravados em USUARIOS / SESSAO_RPG / SESSAO_PLAYERS após confirmar
    login VARCHAR(50) NOT NULL,
    nome VARCHAR(255) NOT NULL,
    email VARCHAR(255) NOT NULL,
    senha_hash VARCHAR(255) NOT NULL,
    papel VARCHAR(100) NOT NULL DEFAULT 'Mestre (Host)',
    sessao_id BIGINT UNSIGNED NULL,
    sessao_nome VARCHAR(255) NOT NULL DEFAULT 'Crônicas de Kharos',
    sessao_sistema VARCHAR(100) NOT NULL DEFAULT 'Old Dragon / D&D 5e',
    sessao_descricao TEXT NULL,
    codigo_convite VARCHAR(100) NULL,
    servidor VARCHAR(255) NOT NULL DEFAULT '127.0.0.1:8080',
    confirmado BOOLEAN NOT NULL DEFAULT FALSE,
    usuario_id BIGINT UNSIGNED NULL,
    expira_em DATETIME(3) NOT NULL,
    confirmado_em DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    UNIQUE KEY uk_verificacoes_token (token),
    INDEX idx_verificacoes_codigo_login (codigo, login),
    INDEX idx_verificacoes_email (email),
    INDEX idx_verificacoes_expira (expira_em)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
