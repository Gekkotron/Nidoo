//! The Home Assistant discovery abbreviation tables, embedded from the app's resource file.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde_json::{Map, Value};

/// The app's table, verbatim: `payload` for entity configs, `device` and `origin` for those objects.
const TABLE_JSON: &str = include_str!("abbreviations.json");

/// Abbreviated key → full key, for one nesting level.
pub type Table = HashMap<String, String>;

pub struct Tables {
    pub payload: Table,
    pub device: Table,
    pub origin: Table,
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let raw: HashMap<String, Table> =
        serde_json::from_str(TABLE_JSON).expect("abbreviations.json is three string maps");
    let take = |name: &str| raw.get(name).cloned().unwrap_or_default();
    Tables {
        payload: take("payload"),
        device: take("device"),
        origin: take("origin"),
    }
});

/// The embedded tables.
pub fn tables() -> &'static Tables {
    &TABLES
}

/// Renames every abbreviated key of `object` to its full name; a full key that is also present
/// wins over its abbreviation.
pub fn rename(object: Map<String, Value>, table: &Table) -> Map<String, Value> {
    let mut out = Map::with_capacity(object.len());
    for (key, value) in object {
        match table.get(&key) {
            Some(full) if *full != key => {
                if !out.contains_key(full) {
                    out.insert(full.clone(), value);
                }
            }
            _ => {
                out.insert(key, value);
            }
        }
    }
    out
}
