//! "Ask" helper: a small offline bot that answers "is what I'm looking for in
//! this table?". No AI involved: it normalises the question (Portuguese or
//! English), drops filler words, expands a few synonyms and matches what is
//! left against table and column names. Quoted text is treated as a value to
//! look for inside the data.

use crate::db::{ColumnInfo, TableInfo};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Pt,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub sid: u64,
    pub conn: String,
    pub table: TableInfo,
    /// (column name, type) when a column matched; None = the table itself
    pub column: Option<(String, String)>,
    pub score: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub text: String,
    pub hits: Vec<Hit>,
    /// text to look for inside the data (quoted text, numbers, or leftover words)
    pub value: Option<String>,
    /// true when the data search should start right away (quoted value)
    pub auto_search: bool,
    pub lang: Option<Lang>,
}

pub struct Candidate<'a> {
    pub sid: u64,
    pub conn: String,
    pub table: &'a TableInfo,
    pub cols: &'a [ColumnInfo],
}

const STOP: &[&str] = &[
    // pt
    "tem", "tem?", "existe", "existem", "onde", "esta", "estao", "fica", "ficam", "a", "o", "os", "as", "de", "da", "do",
    "das", "dos", "na", "no", "nas", "nos", "em", "um", "uma", "uns", "umas", "e", "ou", "que", "qual", "quais", "tabela",
    "tabelas", "coluna", "colunas", "campo", "campos", "procuro", "procurando", "estou", "eu", "ta", "nessa", "nesta",
    "neste", "nesse", "essa", "esse", "este", "isso", "isto", "aqui", "ai", "la", "algum", "alguma", "lugar", "banco",
    "dados", "com", "por", "pra", "para", "sobre", "guarda", "guardo", "salva", "salvo", "achar", "acho", "encontrar",
    "encontro", "cade", "me", "mostra", "mostrar", "quero", "ver", "sei", "nao", "sim", "valor", "informacao", "info",
    "contem", "possui", "tenho", "temos", "ha",
    // en
    "is", "are", "there", "any", "where", "which", "what", "the", "a", "an", "of", "in", "on", "at", "to", "for", "with",
    "table", "tables", "column", "columns", "field", "fields", "this", "that", "these", "those", "have", "has", "does",
    "do", "contain", "contains", "i", "im", "looking", "find", "search", "for", "my", "it", "stored", "store", "kept",
    "anywhere", "database", "data", "some", "can", "you", "show", "me", "please", "value", "info", "and", "or",
];

/// Groups of words that mean the same thing (already without accents).
const SYNONYMS: &[&[&str]] = &[
    &["email", "mail", "correio", "emails"],
    &["phone", "telefone", "fone", "celular", "tel", "mobile", "whatsapp"],
    &["name", "nome", "nomes", "names", "title", "titulo"],
    &["price", "preco", "precos", "valor", "cost", "custo", "unit_price"],
    &["date", "data", "dia", "day", "when", "quando"],
    &["created", "criado", "criacao", "created_at", "cadastro"],
    &["city", "cidade"],
    &["address", "endereco", "rua", "street"],
    &["user", "usuario", "usuarios", "users"],
    &["customer", "cliente", "clientes", "client", "customers"],
    &["order", "pedido", "pedidos", "compra", "compras", "orders"],
    &["product", "produto", "produtos", "item", "itens", "items", "products"],
    &["quantity", "quantidade", "qtd", "qty"],
    &["total", "soma", "amount", "montante"],
    &["status", "situacao", "estado", "state"],
    &["country", "pais", "paises"],
    &["stock", "estoque", "inventory"],
    &["category", "categoria", "categorias", "tipo", "type"],
    &["id", "codigo", "code", "identificador"],
    &["password", "senha"],
    &["birth", "nascimento", "aniversario", "birthday"],
    &["zip", "cep", "postal"],
    &["description", "descricao"],
    &["discount", "desconto"],
    &["payment", "pagamento", "pagamentos"],
    &["company", "empresa", "empresas"],
    &["document", "documento", "cpf", "cnpj"],
    &["tag", "tags", "etiqueta", "etiquetas"],
    &["plan", "plano", "planos"],
    &["weight", "peso", "weight_g"],
    &["revenue", "receita", "faturamento", "vendas", "sales"],
    &["profile", "perfil"],
    &["newsletter", "boletim"],
];

fn strip_accents(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

pub fn detect_lang(q: &str) -> Lang {
    let n = strip_accents(&q.to_lowercase());
    let pt = ["tem ", "onde", "esta", "tabela", "coluna", "qual", "existe", "nessa", "algum", "cade", "procuro", " ta ", "campo"];
    if pt.iter().any(|w| n.contains(w)) || q.chars().any(|c| "ãõçáéíóúê".contains(c)) {
        Lang::Pt
    } else {
        Lang::En
    }
}

/// Text between quotes ('…', "…", “…”), if any.
fn quoted(q: &str) -> Option<String> {
    for (open, close) in [('"', '"'), ('\'', '\''), ('“', '”'), ('‘', '’')] {
        if let Some(a) = q.find(open) {
            let rest = &q[a + open.len_utf8()..];
            if let Some(b) = rest.find(close) {
                let v = rest[..b].trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn words(q: &str) -> Vec<String> {
    strip_accents(&q.to_lowercase())
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '@' || c == '.' || c == '-'))
        .map(|w| w.trim_matches(|c: char| c == '.' || c == '-').to_string())
        .filter(|w| w.chars().count() >= 2 && !STOP.contains(&w.as_str()))
        .collect()
}

fn singular(w: &str) -> String {
    if w.len() > 4 && w.ends_with("oes") {
        format!("{}ao", &w[..w.len() - 3])
    } else if w.len() > 3 && w.ends_with("es") && !w.ends_with("ses") {
        w[..w.len() - 2].to_string()
    } else if w.len() > 3 && w.ends_with('s') {
        w[..w.len() - 1].to_string()
    } else {
        w.to_string()
    }
}

/// The word plus its singular form and synonyms.
pub fn variants(w: &str) -> Vec<String> {
    let mut v = vec![w.to_string()];
    let s = singular(w);
    if s != w {
        v.push(s.clone());
    }
    for g in SYNONYMS {
        if g.iter().any(|x| *x == w || *x == s) {
            for x in g.iter() {
                if !v.iter().any(|y| y == x) {
                    v.push(x.to_string());
                }
            }
        }
    }
    v
}

fn name_parts(name: &str) -> Vec<String> {
    // split snake_case and camelCase
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c == '_' || c == '-' || c == ' ' || c == '.' {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_lowercase();
        cur.push(c);
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts.into_iter().map(|p| strip_accents(&p.to_lowercase())).collect()
}

/// How well a name matches any of the variants (0 = no match).
fn score_name(name: &str, vars: &[String]) -> u32 {
    let full = strip_accents(&name.to_lowercase());
    let parts = name_parts(name);
    let mut best = 0;
    for (i, v) in vars.iter().enumerate() {
        // the word as typed counts a bit more than a synonym
        let bonus = if i == 0 { 10 } else { 0 };
        let s = if full == *v || singular(&full) == *v {
            120
        } else if parts.iter().any(|p| p == v || singular(p) == *v) {
            100
        } else if v.len() >= 3 && parts.iter().any(|p| p.starts_with(v.as_str())) {
            70
        } else if v.len() >= 4 && full.contains(v.as_str()) {
            50
        } else {
            0
        };
        if s > 0 {
            best = best.max(s + bonus);
        }
    }
    best
}

fn short_type(t: &str) -> String {
    t.to_lowercase()
}

/// Answer a question about the schema. `focus` = (connection id, table name)
/// when the user is asking about one table.
pub fn answer(question: &str, cands: &[Candidate], focus: Option<(u64, String)>) -> Reply {
    let lang = detect_lang(question);
    let pt = lang == Lang::Pt;
    let q = question.trim();
    let mut reply = Reply { lang: Some(lang), ..Default::default() };
    if q.is_empty() {
        reply.text = if pt {
            "Me pergunte, por exemplo: “tem email aqui?”, “onde fica o preço?” ou “tem 'Honey Jar' em algum lugar?”".into()
        } else {
            "Ask me things like: “is there an email column?”, “where is the price?” or “is 'Honey Jar' anywhere?”".into()
        };
        return reply;
    }
    let value = quoted(q);
    let rest = match &value {
        Some(v) => q.replace(v.as_str(), " "),
        None => q.to_string(),
    };
    let ws = words(&rest);
    let mut hits: Vec<Hit> = Vec::new();
    for w in &ws {
        let vars = variants(w);
        for c in cands {
            let ts = score_name(&c.table.name, &vars);
            if ts > 0 {
                hits.push(Hit { sid: c.sid, conn: c.conn.clone(), table: c.table.clone(), column: None, score: ts + 5 });
            }
            for col in c.cols {
                let s = score_name(&col.name, &vars);
                if s > 0 {
                    hits.push(Hit { sid: c.sid, conn: c.conn.clone(), table: c.table.clone(), column: Some((col.name.clone(), short_type(&col.data_type))), score: s });
                }
            }
        }
    }
    // merge duplicates, keep the best score
    hits.sort_by(|a, b| b.score.cmp(&a.score));
    let mut seen = std::collections::HashSet::new();
    hits.retain(|h| seen.insert((h.sid, h.table.key(), h.column.as_ref().map(|c| c.0.clone()))));
    // numbers alone are values, not names
    let numeric_value = ws.iter().find(|w| w.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',')).cloned();
    reply.value = value.clone().or(numeric_value.clone());

    let in_focus: Vec<Hit> = match &focus {
        Some((sid, t)) => hits.iter().filter(|h| h.sid == *sid && h.table.name == *t).cloned().collect(),
        None => vec![],
    };
    let describe = |h: &Hit| -> String {
        match &h.column {
            Some((c, ty)) => {
                if pt {
                    format!("coluna `{c}` ({ty}) na tabela `{}`", h.table.name)
                } else {
                    format!("column `{c}` ({ty}) in table `{}`", h.table.name)
                }
            }
            None => {
                if pt {
                    format!("a tabela `{}`", h.table.name)
                } else {
                    format!("the table `{}`", h.table.name)
                }
            }
        }
    };

    if let Some(v) = &value {
        reply.auto_search = true;
        reply.text = if pt { format!("Vou procurar “{v}” dentro dos dados…") } else { format!("Looking for “{v}” inside the data…") };
        reply.hits = hits.into_iter().take(6).collect();
        return reply;
    }
    if ws.is_empty() {
        reply.text = if pt { "Não entendi o que procurar. Diga um nome, tipo “email” ou “preço”.".into() } else { "I couldn't tell what to look for. Try a word like “email” or “price”.".into() };
        return reply;
    }
    let what = ws.join(" ");
    if let Some((_, t)) = &focus {
        if !in_focus.is_empty() {
            let first = describe(&in_focus[0]);
            reply.text = if pt { format!("✅ Sim! Achei {first}.") } else { format!("✅ Yes! Found {first}.") };
            let mut list = in_focus;
            list.extend(hits.into_iter().filter(|h| !(h.table.name == *t)).take(3));
            reply.hits = list.into_iter().take(6).collect();
            return reply;
        }
        if !hits.is_empty() {
            reply.text = if pt {
                format!("❌ Não está em `{t}`. Mas achei em outro lugar:")
            } else {
                format!("❌ Not in `{t}`. But I found it elsewhere:")
            };
            reply.hits = hits.into_iter().take(6).collect();
            return reply;
        }
    } else if !hits.is_empty() {
        let n_tables = {
            let mut s = std::collections::HashSet::new();
            for h in &hits {
                s.insert((h.sid, h.table.key()));
            }
            s.len()
        };
        reply.text = if pt {
            format!("✅ Achei “{what}” em {n_tables} tabela(s). O mais provável: {}.", describe(&hits[0]))
        } else {
            format!("✅ Found “{what}” in {n_tables} table(s). Best match: {}.", describe(&hits[0]))
        };
        reply.hits = hits.into_iter().take(8).collect();
        return reply;
    }
    reply.value = Some(numeric_value.unwrap_or(what.clone()));
    reply.text = if pt {
        format!("🤷 Nenhuma tabela ou coluna se chama “{what}”. Quer que eu procure isso dentro dos dados?")
    } else {
        format!("🤷 No table or column is called “{what}”. Want me to look for it inside the data?")
    };
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(names: &[(&str, &str)]) -> Vec<ColumnInfo> {
        names.iter().map(|(n, t)| ColumnInfo { name: n.to_string(), data_type: t.to_string(), nullable: true, pk: false }).collect()
    }

    #[test]
    fn finds_columns_in_portuguese() {
        let t1 = TableInfo { schema: None, name: "customers".into(), is_view: false };
        let t2 = TableInfo { schema: None, name: "products".into(), is_view: false };
        let c1 = cols(&[("id", "INTEGER"), ("email", "TEXT"), ("created_at", "TEXT")]);
        let c2 = cols(&[("id", "INTEGER"), ("price", "REAL")]);
        let cands = vec![
            Candidate { sid: 1, conn: "demo".into(), table: &t1, cols: &c1 },
            Candidate { sid: 1, conn: "demo".into(), table: &t2, cols: &c2 },
        ];
        let r = answer("onde fica o preço?", &cands, None);
        assert_eq!(r.hits[0].column.as_ref().unwrap().0, "price");
        let r = answer("tem email nessa tabela?", &cands, Some((1, "products".into())));
        assert!(r.text.starts_with("❌"));
        assert_eq!(r.hits[0].table.name, "customers");
        let r = answer("is there an email column?", &cands, Some((1, "customers".into())));
        assert!(r.text.starts_with("✅"));
        let r = answer("tem 'Honey Jar' em algum lugar?", &cands, None);
        assert_eq!(r.value.as_deref(), Some("Honey Jar"));
        assert!(r.auto_search);
        let r = answer("cadê os clientes?", &cands, None);
        assert_eq!(r.hits[0].table.name, "customers");
        let r = answer("xyzzy", &cands, None);
        assert!(r.hits.is_empty() && r.value.is_some());
    }
}
