//! 已解析的 MasterData 表。
//!
//! JSON 数组解析为 `Vec<Value>` 并对 `id` 字段建索引，O(1) 查找。

use serde_json::Value;
use std::collections::HashMap;

/// 一张已解析的 MasterData 表。
pub struct Table {
    records: Vec<Value>,
    id_index: HashMap<i64, usize>,
}

impl Table {
    /// 从 JSON 字符串解析。数组建表并索引 `id` 字段；
    /// 单个对象作为一条记录存入。
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

    /// 按 id 查找记录。
    pub fn by_id(&self, id: i64) -> Option<&Value> {
        self.id_index.get(&id).map(|&i| &self.records[i])
    }

    /// 全部记录。
    pub fn all(&self) -> &[Value] {
        &self.records
    }

    /// 记录数量。
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// 按 id 反序列化为强类型记录。
    pub fn typed<T: serde::de::DeserializeOwned>(&self, id: i64) -> Option<T> {
        serde_json::from_value(self.by_id(id)?.clone()).ok()
    }
}
