-- Script executado automaticamente na primeira inicialização do volume do MariaDB.
--
-- O banco (MARIADB_DATABASE) e o usuário da aplicação (MARIADB_USER, com todos os
-- privilégios nesse banco) já são criados pela imagem oficial do MariaDB a partir do .env.
--
-- As tabelas NÃO são criadas aqui: a fonte única do schema são as migrations da API
-- (RpgDBCrud/migrations), aplicadas automaticamente quando a API sobe.
SET NAMES utf8mb4;
