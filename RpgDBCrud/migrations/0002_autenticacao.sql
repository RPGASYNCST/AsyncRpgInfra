-- Autenticação da API: usuários, sessões de login, dispositivos confiáveis e penalidades
-- por tentativa de login errada.

CREATE TABLE IF NOT EXISTS USUARIOS (
    id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    -- Sempre gravado em minúsculas; único.
    login VARCHAR(50) NOT NULL,
    nome VARCHAR(255) NOT NULL,
    -- Hash Argon2id no formato PHC; a senha em texto nunca é gravada.
    senha_hash VARCHAR(255) NOT NULL,
    admin BOOLEAN NOT NULL DEFAULT FALSE,
    ativo BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    PRIMARY KEY (id),
    UNIQUE KEY uk_usuarios_login (login)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Token de sessão entregue no login. Só o SHA-256 do token é gravado.
CREATE TABLE IF NOT EXISTS SESSOES_AUTH (
    token_hash BINARY(32) NOT NULL,
    usuario_id BIGINT UNSIGNED NOT NULL,
    expira_em DATETIME(3) NOT NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    PRIMARY KEY (token_hash),
    INDEX idx_sessoes_auth_usuario (usuario_id),
    INDEX idx_sessoes_auth_expira (expira_em),
    CONSTRAINT fk_sessoes_auth_usuario
        FOREIGN KEY (usuario_id) REFERENCES USUARIOS (id)
        ON DELETE CASCADE ON UPDATE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Dispositivo confiável: entregue após um login correto e guardado pelo app. Logins
-- vindos dele não sofrem a penalidade da conta (só a do próprio dispositivo), então um
-- atacante errando a senha não bloqueia o dono da conta no computador de sempre.
CREATE TABLE IF NOT EXISTS DISPOSITIVOS (
    id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    usuario_id BIGINT UNSIGNED NOT NULL,
    token_hash BINARY(32) NOT NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    ultimo_uso_em DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    UNIQUE KEY uk_dispositivos_token (token_hash),
    INDEX idx_dispositivos_usuario (usuario_id),
    CONSTRAINT fk_dispositivos_usuario
        FOREIGN KEY (usuario_id) REFERENCES USUARIOS (id)
        ON DELETE CASCADE ON UPDATE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Penalidades exponenciais. Chaves: 'ip:<ip>', 'conta:<login>', 'disp:<id>', 'cadastro:<ip>'.
CREATE TABLE IF NOT EXISTS PENALIDADES (
    chave VARCHAR(191) NOT NULL,
    falhas INT UNSIGNED NOT NULL DEFAULT 0,
    bloqueado_ate DATETIME(3) NULL,
    ultima_falha_em DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    PRIMARY KEY (chave),
    INDEX idx_penalidades_ultima_falha (ultima_falha_em)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
