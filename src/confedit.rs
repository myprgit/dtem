//! Comment-preserving edits of dtem.toml (used by the settings page and the tilda importer).
use crate::config::config_path;
use toml_edit::{DocumentMut, Item, Table};

pub fn load_doc() -> DocumentMut {
    std::fs::read_to_string(config_path()).ok().and_then(|s| s.parse::<DocumentMut>().ok()).unwrap_or_default()
}

pub fn save_doc(doc: &DocumentMut) -> std::io::Result<()> {
    crate::session::write_atomic(&config_path(), doc.to_string().as_bytes())
}

/// Set `a.b.c` (tables are created as needed).
pub fn set(doc: &mut DocumentMut, path: &str, v: impl Into<toml_edit::Value>) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut table: &mut Table = doc.as_table_mut();
    for p in &parts[..parts.len() - 1] {
        if !table.contains_key(p) || !table[p].is_table() {
            let mut t = Table::new();
            t.set_implicit(false);
            table.insert(p, Item::Table(t));
        }
        table = table[p].as_table_mut().unwrap();
    }
    let key = parts[parts.len() - 1];
    let mut value: toml_edit::Value = v.into();
    // keep an existing trailing comment on the line
    if let Some(old) = table.get(key).and_then(|i| i.as_value()) {
        let decor = old.decor().clone();
        *value.decor_mut() = decor;
    }
    table.insert(key, Item::Value(value));
}

pub fn remove(doc: &mut DocumentMut, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut table: &mut Table = doc.as_table_mut();
    for p in &parts[..parts.len() - 1] {
        match table.get_mut(p).and_then(|i| i.as_table_mut()) {
            Some(t) => table = t,
            None => return,
        }
    }
    table.remove(parts[parts.len() - 1]);
}

pub fn string_array(v: &[String]) -> toml_edit::Value {
    let mut a = toml_edit::Array::new();
    for s in v {
        a.push(s.as_str());
    }
    toml_edit::Value::Array(a)
}
