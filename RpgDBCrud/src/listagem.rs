//! Listagem padronizada: paginação, busca textual, ordenação e filtros.
//!
//! Parâmetros aceitos por todas as rotas `GET /api/{tabela}`:
//!
//! | Parâmetro      | Exemplo                      | Efeito                                         |
//! | :------------- | :--------------------------- | :--------------------------------------------- |
//! | `limit`        | `limit=20`                   | Itens por página (padrão 50, máximo 200)       |
//! | `offset`       | `offset=40`                  | Quantos itens pular                            |
//! | `q`            | `q=dragao`                   | Busca parcial nas colunas de texto da tabela   |
//! | `order_by`     | `order_by=name`              | Coluna de ordenação (lista fechada por tabela) |
//! | `order`        | `order=asc`                  | `asc` ou `desc` (padrão `desc`)                |
//! | `created_from` | `created_from=2026-11-01T00:00:00Z` | Criados a partir da data                |
//! | `created_to`   | `created_to=2026-11-30T23:59:59Z`   | Criados até a data                      |
//! | `updated_from` | `updated_from=2026-11-11T18:00:00Z` | Alterados desde a data (sincronização)  |
//!
//! Cada tabela pode somar filtros exatos próprios (ex.: `sessao_id`).
//! Nomes de coluna nunca vêm do cliente: só de listas fixas no código; valores sempre
//! entram por bind, então não há risco de SQL injection.

use crate::error::AppError;
use crate::validacao::Validador;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::mysql::MySqlRow;
use sqlx::{FromRow, MySql, MySqlPool, QueryBuilder};

pub const LIMITE_PADRAO: u32 = 50;
pub const LIMITE_MAXIMO: u32 = 200;
pub const BUSCA_MAX_CHARS: usize = 100;

/// Parâmetros comuns de listagem (query string).
#[derive(Debug, Default, Deserialize)]
pub struct ParametrosLista {
    pub limit: Option<u32>,
    pub offset: Option<u64>,
    pub q: Option<String>,
    pub order_by: Option<String>,
    pub order: Option<String>,
    pub created_from: Option<DateTime<Utc>>,
    pub created_to: Option<DateTime<Utc>>,
    pub updated_from: Option<DateTime<Utc>>,
}

/// Resposta paginada.
#[derive(Debug, Serialize)]
pub struct Pagina<T> {
    pub items: Vec<T>,
    /// Total de registros que atendem aos filtros (sem paginação).
    pub total: i64,
    pub limit: u32,
    pub offset: u64,
}

/// Descrição fixa de uma tabela para a listagem.
pub struct Tabela {
    pub nome: &'static str,
    /// Lista de colunas do SELECT.
    pub colunas: &'static str,
    /// Colunas pesquisadas pelo parâmetro `q` (com `LIKE`).
    pub busca: &'static [&'static str],
    /// Pares (nome aceito em `order_by`, coluna SQL).
    pub ordenaveis: &'static [(&'static str, &'static str)],
}

/// Valor de um filtro exato.
#[derive(Debug, Clone)]
pub enum Valor {
    Id(u64),
    Texto(String),
}

/// Filtro exato `coluna = valor`; a coluna vem sempre de constante no código.
pub type Filtro = (&'static str, Valor);

/// Parâmetros já validados.
#[derive(Debug)]
struct Consulta {
    limit: u32,
    offset: u64,
    padrao_busca: Option<String>,
    coluna_ordem: &'static str,
    direcao: &'static str,
    created_from: Option<DateTime<Utc>>,
    created_to: Option<DateTime<Utc>>,
    updated_from: Option<DateTime<Utc>>,
}

fn validar(tabela: &Tabela, p: &ParametrosLista) -> Result<Consulta, AppError> {
    let mut v = Validador::new();

    let limit = p.limit.unwrap_or(LIMITE_PADRAO);
    if limit == 0 || limit > LIMITE_MAXIMO {
        v.erro("limit", format!("deve estar entre 1 e {LIMITE_MAXIMO}"));
    }

    let padrao_busca = match p.q.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(q) if q.chars().count() > BUSCA_MAX_CHARS => {
            v.erro(
                "q",
                format!("excede o limite de {BUSCA_MAX_CHARS} caracteres"),
            );
            None
        }
        Some(_) if tabela.busca.is_empty() => {
            v.erro("q", "esta tabela não aceita busca textual");
            None
        }
        Some(q) => Some(format!("%{}%", escapar_like(q))),
    };

    let coluna_ordem = match p.order_by.as_deref() {
        None => "id",
        Some(nome) => match tabela.ordenaveis.iter().find(|(n, _)| *n == nome) {
            Some((_, coluna)) => coluna,
            None => {
                let aceitos: Vec<&str> = tabela.ordenaveis.iter().map(|(n, _)| *n).collect();
                v.erro("order_by", format!("use um de: {}", aceitos.join(", ")));
                "id"
            }
        },
    };

    let direcao = match p.order.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("desc") => "DESC",
        Some("asc") => "ASC",
        Some(_) => {
            v.erro("order", "use 'asc' ou 'desc'");
            "DESC"
        }
    };

    if let (Some(de), Some(ate)) = (p.created_from, p.created_to) {
        if de > ate {
            v.erro("created_from", "não pode ser maior que 'created_to'");
        }
    }

    v.finalizar()?;
    Ok(Consulta {
        limit,
        offset: p.offset.unwrap_or(0),
        padrao_busca,
        coluna_ordem,
        direcao,
        created_from: p.created_from,
        created_to: p.created_to,
        updated_from: p.updated_from,
    })
}

/// Escapa os curingas do LIKE (`\`, `%`, `_`) para a busca ser literal.
fn escapar_like(texto: &str) -> String {
    let mut saida = String::with_capacity(texto.len());
    for c in texto.chars() {
        if matches!(c, '\\' | '%' | '_') {
            saida.push('\\');
        }
        saida.push(c);
    }
    saida
}

fn montar_where<'a>(
    qb: &mut QueryBuilder<'a, MySql>,
    tabela: &Tabela,
    c: &Consulta,
    filtros: &[Filtro],
) {
    let mut sep = " WHERE ";
    for (coluna, valor) in filtros {
        qb.push(sep).push(*coluna).push(" = ");
        match valor {
            Valor::Id(id) => qb.push_bind(*id),
            Valor::Texto(t) => qb.push_bind(t.clone()),
        };
        sep = " AND ";
    }
    if let Some(padrao) = &c.padrao_busca {
        qb.push(sep).push("(");
        for (i, coluna) in tabela.busca.iter().enumerate() {
            if i > 0 {
                qb.push(" OR ");
            }
            qb.push(*coluna).push(" LIKE ").push_bind(padrao.clone());
        }
        qb.push(")");
        sep = " AND ";
    }
    if let Some(data) = c.created_from {
        qb.push(sep).push("created_at >= ").push_bind(data);
        sep = " AND ";
    }
    if let Some(data) = c.created_to {
        qb.push(sep).push("created_at <= ").push_bind(data);
        sep = " AND ";
    }
    if let Some(data) = c.updated_from {
        qb.push(sep).push("updated_at >= ").push_bind(data);
    }
}

fn montar_select<'a>(tabela: &Tabela, c: &Consulta, filtros: &[Filtro]) -> QueryBuilder<'a, MySql> {
    let mut qb = QueryBuilder::new(format!("SELECT {} FROM {}", tabela.colunas, tabela.nome));
    montar_where(&mut qb, tabela, c, filtros);
    // `id` como desempate deixa a paginação estável quando a coluna de ordem se repete.
    qb.push(format!(
        " ORDER BY {} {}, id {} LIMIT ",
        c.coluna_ordem, c.direcao, c.direcao
    ));
    qb.push_bind(c.limit).push(" OFFSET ").push_bind(c.offset);
    qb
}

fn montar_count<'a>(tabela: &Tabela, c: &Consulta, filtros: &[Filtro]) -> QueryBuilder<'a, MySql> {
    let mut qb = QueryBuilder::new(format!("SELECT COUNT(*) FROM {}", tabela.nome));
    montar_where(&mut qb, tabela, c, filtros);
    qb
}

/// Executa a listagem paginada.
pub async fn listar<T>(
    pool: &MySqlPool,
    tabela: &Tabela,
    params: &ParametrosLista,
    filtros: &[Filtro],
) -> Result<Pagina<T>, AppError>
where
    T: for<'r> FromRow<'r, MySqlRow> + Send + Unpin,
{
    let consulta = validar(tabela, params)?;

    let total: i64 = montar_count(tabela, &consulta, filtros)
        .build_query_scalar()
        .fetch_one(pool)
        .await?;
    let items: Vec<T> = montar_select(tabela, &consulta, filtros)
        .build_query_as()
        .fetch_all(pool)
        .await?;

    Ok(Pagina {
        items,
        total,
        limit: consulta.limit,
        offset: consulta.offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABELA: Tabela = Tabela {
        nome: "SESSAO_RPG",
        colunas: "id, name",
        busca: &["name", "description"],
        ordenaveis: &[("id", "id"), ("name", "name"), ("system", "`system`")],
    };

    fn sql(p: &ParametrosLista, filtros: &[Filtro]) -> String {
        let c = validar(&TABELA, p).expect("parâmetros válidos");
        montar_select(&TABELA, &c, filtros).sql().to_string()
    }

    #[test]
    fn padrao_ordena_por_id_desc_com_limite() {
        assert_eq!(
            sql(&ParametrosLista::default(), &[]),
            "SELECT id, name FROM SESSAO_RPG ORDER BY id DESC, id DESC LIMIT ? OFFSET ?"
        );
    }

    #[test]
    fn filtros_busca_e_datas_viram_binds() {
        let p = ParametrosLista {
            q: Some("drag".into()),
            order_by: Some("system".into()),
            order: Some("ASC".into()),
            created_from: Some(Utc::now()),
            ..Default::default()
        };
        assert_eq!(
            sql(&p, &[("sessao_id", Valor::Id(1))]),
            "SELECT id, name FROM SESSAO_RPG WHERE sessao_id = ? AND (name LIKE ? OR description LIKE ?) \
             AND created_at >= ? ORDER BY `system` ASC, id ASC LIMIT ? OFFSET ?"
        );
    }

    #[test]
    fn ordem_fora_da_lista_e_limite_invalido_falham() {
        let p = ParametrosLista {
            limit: Some(1000),
            order_by: Some("name; DROP TABLE SESSAO_RPG".into()),
            order: Some("lado".into()),
            ..Default::default()
        };
        match validar(&TABELA, &p) {
            Err(AppError::Validacao(erros)) => {
                let campos: Vec<_> = erros.iter().map(|e| e.campo.as_str()).collect();
                assert_eq!(campos, vec!["limit", "order_by", "order"]);
            }
            outro => panic!("esperava erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn busca_escapa_curingas() {
        assert_eq!(escapar_like(r"50%_off\"), r"50\%\_off\\");
    }

    #[test]
    fn busca_vazia_e_ignorada() {
        let p = ParametrosLista {
            q: Some("   ".into()),
            ..Default::default()
        };
        assert!(!sql(&p, &[]).contains("LIKE"));
    }
}
