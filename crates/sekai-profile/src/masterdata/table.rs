//! A parsed masterdata table.
//!
//! A JSON array is parsed into `Vec<Value>` with an index on the `id` field,
//! so lookups by ID take constant time.

use serde_json::Value;
use std::collections::HashMap;

/// A parsed masterdata table.
pub struct Table {
    records: Vec<Value>,
    id_index: HashMap<i64, usize>,
}

impl Table {
    /// Parses a table from JSON text. An array becomes the rows, indexed by
    /// their `id` field; any other value is stored as a single row.
    ///
    /// # Errors
    ///
    /// Returns the parser error when `json` is not valid JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let raw: Value = serde_json::from_str(json)?;
        let records = match raw {
            Value::Array(arr) => arr,
            other => vec![other],
        };

        let mut id_index = HashMap::new();
        for (i, record) in records.iter().enumerate() {
            if let Some(id) = record.get("id").and_then(|v| v.as_i64()) {
                id_index.insert(id, i);
            }
        }
        Ok(Self { records, id_index })
    }

    /// The row whose `id` is `id`.
    pub fn by_id(&self, id: i64) -> Option<&Value> {
        self.id_index.get(&id).map(|&i| &self.records[i])
    }

    /// All rows, in source order.
    pub fn all(&self) -> &[Value] {
        &self.records
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the table has no rows.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Deserializes the row `id` into `T`.
    ///
    /// Returns `None` when the row is missing. A row that exists but does not
    /// match `T` also returns `None` and logs a warning, so a data defect can
    /// be told apart from a missing ID.
    pub fn typed<T: serde::de::DeserializeOwned>(&self, id: i64) -> Option<T> {
        let value = self.by_id(id)?;
        match serde_json::from_value(value.clone()) {
            Ok(typed) => Some(typed),
            Err(error) => {
                tracing::warn!(%id, %error, "masterdata row does not deserialize; treated as missing");
                None
            }
        }
    }
}
