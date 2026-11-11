//! Validações de entrada reutilizáveis.
//!
//! Aqui só entram checagens de formato e de limite físico das colunas (o que o banco
//! rejeitaria ou truncaria). Regras de negócio do jogo NÃO pertencem a esta API.
//!
//! Uso nos handlers:
//! ```ignore
//! let mut v = Validador::new();
//! v.texto_obrigatorio("name", &payload.name, 255);
//! v.texto_opcional("description", payload.description.as_deref(), TEXT_MAX_BYTES);
//! v.finalizar()?; // devolve 422 com a lista de campos inválidos
//! ```

use crate::error::AppError;
use serde::Serialize;
use serde_json::Value;

/// Tamanho máximo de uma coluna `TEXT` do MariaDB, em bytes.
pub const TEXT_MAX_BYTES: usize = 65_535;

/// Tamanho máximo aceito para o JSON de uma ficha, em bytes.
pub const FICHA_MAX_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ErroCampo {
    pub campo: String,
    pub mensagem: String,
}

#[derive(Debug, Default)]
pub struct Validador {
    erros: Vec<ErroCampo>,
}

impl Validador {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn erro(&mut self, campo: &str, mensagem: impl Into<String>) {
        self.erros.push(ErroCampo {
            campo: campo.to_string(),
            mensagem: mensagem.into(),
        });
    }

    /// Texto obrigatório para colunas `VARCHAR(max_chars)`: não vazio, sem espaços nas
    /// pontas, sem caracteres de controle e dentro do limite de caracteres.
    pub fn texto_obrigatorio(&mut self, campo: &str, valor: &str, max_chars: usize) {
        if valor.trim().is_empty() {
            self.erro(campo, "é obrigatório e não pode ser vazio");
            return;
        }
        self.texto_curto(campo, valor, max_chars);
    }

    /// Mesmo que [`Self::texto_obrigatorio`], mas só valida quando o campo foi enviado.
    pub fn texto_obrigatorio_se_enviado(
        &mut self,
        campo: &str,
        valor: Option<&str>,
        max_chars: usize,
    ) {
        if let Some(valor) = valor {
            self.texto_obrigatorio(campo, valor, max_chars);
        }
    }

    /// Texto livre opcional para colunas `TEXT`: aceita quebras de linha e é limitado em bytes.
    pub fn texto_opcional(&mut self, campo: &str, valor: Option<&str>, max_bytes: usize) {
        let Some(valor) = valor else { return };
        if valor.len() > max_bytes {
            self.erro(campo, format!("excede o limite de {max_bytes} bytes"));
        }
        if valor
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            self.erro(campo, "contém caracteres de controle não permitidos");
        }
    }

    /// Identificador de outra tabela: precisa ser maior que zero.
    pub fn id(&mut self, campo: &str, valor: u64) {
        if valor == 0 {
            self.erro(campo, "deve ser maior que zero");
        }
    }

    pub fn id_se_enviado(&mut self, campo: &str, valor: Option<u64>) {
        if let Some(valor) = valor {
            self.id(campo, valor);
        }
    }

    /// JSON que precisa ser um objeto (`{...}`) e caber no limite informado.
    pub fn json_objeto(&mut self, campo: &str, valor: &Value, max_bytes: usize) {
        if !valor.is_object() {
            self.erro(campo, "deve ser um objeto JSON ({ ... })");
            return;
        }
        let tamanho = serde_json::to_vec(valor)
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        if tamanho > max_bytes {
            self.erro(campo, format!("excede o limite de {max_bytes} bytes"));
        }
    }

    /// Exige ao menos um campo preenchido (usado no PUT parcial).
    pub fn algum_campo(&mut self, enviados: &[bool]) {
        if !enviados.iter().any(|e| *e) {
            self.erro("*", "informe ao menos um campo para atualizar");
        }
    }

    /// Encerra a validação: `Ok(())` se não houve erro, senão `AppError::Validacao` (422).
    pub fn finalizar(self) -> Result<(), AppError> {
        if self.erros.is_empty() {
            Ok(())
        } else {
            Err(AppError::Validacao(self.erros))
        }
    }

    fn texto_curto(&mut self, campo: &str, valor: &str, max_chars: usize) {
        if valor.trim() != valor {
            self.erro(campo, "não pode começar nem terminar com espaços");
        }
        if valor.chars().count() > max_chars {
            self.erro(campo, format!("excede o limite de {max_chars} caracteres"));
        }
        if valor.chars().any(char::is_control) {
            self.erro(campo, "contém caracteres de controle não permitidos");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn campos(v: Validador) -> Vec<String> {
        match v.finalizar() {
            Ok(()) => vec![],
            Err(AppError::Validacao(erros)) => erros.into_iter().map(|e| e.campo).collect(),
            Err(outro) => panic!("erro inesperado: {outro:?}"),
        }
    }

    #[test]
    fn texto_valido_passa() {
        let mut v = Validador::new();
        v.texto_obrigatorio("name", "Mesa de sexta", 255);
        assert!(campos(v).is_empty());
    }

    #[test]
    fn texto_vazio_espacos_e_controle_falham() {
        let mut v = Validador::new();
        v.texto_obrigatorio("a", "   ", 10);
        v.texto_obrigatorio("b", " x", 10);
        v.texto_obrigatorio("c", "x\u{0007}", 10);
        assert_eq!(campos(v), vec!["a", "b", "c"]);
    }

    #[test]
    fn limite_conta_caracteres_e_nao_bytes() {
        let mut v = Validador::new();
        v.texto_obrigatorio("name", "ação", 4);
        assert!(campos(v).is_empty());
        let mut v = Validador::new();
        v.texto_obrigatorio("name", "açãoo", 4);
        assert_eq!(campos(v), vec!["name"]);
    }

    #[test]
    fn texto_opcional_aceita_quebra_de_linha_e_limita_bytes() {
        let mut v = Validador::new();
        v.texto_opcional("description", Some("linha 1\nlinha 2"), 100);
        v.texto_opcional("resume", None, 1);
        assert!(campos(v).is_empty());
        let mut v = Validador::new();
        v.texto_opcional("description", Some("ab"), 1);
        assert_eq!(campos(v), vec!["description"]);
    }

    #[test]
    fn id_zero_e_json_nao_objeto_falham() {
        let mut v = Validador::new();
        v.id("sessao_id", 0);
        v.json_objeto("sheet", &json!([1, 2]), 100);
        v.json_objeto("sheet2", &json!({"forca": 10}), 5);
        assert_eq!(campos(v), vec!["sessao_id", "sheet", "sheet2"]);
    }

    #[test]
    fn update_sem_campos_falha() {
        let mut v = Validador::new();
        v.algum_campo(&[false, false]);
        assert_eq!(campos(v), vec!["*"]);
    }
}
