//! 渲染引擎内部资源提供者。
//!
//! 与应用层解耦，只负责字节缓存与可选磁盘缓存。
//! 字节预算驱逐：当 `current_bytes + new_data > max_bytes` 时，LRU 驱逐旧条目释放空间。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lru::LruCache;

/// 引擎内部资源缓存。
///
/// 按 key 缓存字节数据，带字节预算的 LRU 驱逐。
pub struct ResourceProvider {
    cache: Mutex<LruCache<String, Arc<Vec<u8>>>>,
    /// 当前缓存总字节数。
    current_bytes: Mutex<usize>,
    /// 最大缓存字节数（硬上限）。
    max_bytes: usize,
    disk_cache_dir: Option<PathBuf>,
}

impl ResourceProvider {
    /// 创建指定大小的资源缓存。
    ///
    /// `max_mb` 为最大缓存大小（MB），超限时按 LRU 驱逐旧条目。
    pub fn new(max_mb: usize) -> Self {
        Self::with_max_bytes(max_mb * 1024 * 1024)
    }

    /// 创建指定字节数的资源缓存。
    pub fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            cache: Mutex::new(LruCache::unbounded()),
            current_bytes: Mutex::new(0),
            max_bytes,
            disk_cache_dir: None,
        }
    }

    /// 设置磁盘缓存目录。
    pub fn set_disk_cache_dir(&mut self, dir: PathBuf) {
        std::fs::create_dir_all(&dir).ok();
        self.disk_cache_dir = Some(dir);
    }

    /// 从缓存获取资源字节（LRU → 磁盘回退）。
    pub fn get(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(v) = cache.get(key).cloned() {
                return Some(v);
            }
        }

        if let Some(ref dir) = self.disk_cache_dir {
            let path = dir.join(key.replace('/', "__"));
            if let Ok(data) = std::fs::read(&path) {
                let data_len = data.len();
                let arc = Arc::new(data);
                let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                let mut bytes = self.current_bytes.lock().unwrap_or_else(|e| e.into_inner());
                while *bytes + data_len > self.max_bytes {
                    match cache.pop_lru() {
                        Some((_, evicted)) => {
                            *bytes = bytes.saturating_sub(evicted.len());
                        }
                        None => break,
                    }
                }
                cache.put(key.to_string(), arc.clone());
                *bytes += data_len;
                return Some(arc);
            }
        }

        None
    }

    /// 将资源放入缓存，超字节预算时按 LRU 驱逐旧条目。
    pub fn put(&self, key: String, data: Vec<u8>) {
        if let Some(ref dir) = self.disk_cache_dir {
            let path = dir.join(key.replace('/', "__"));
            std::fs::write(&path, &data).ok();
        }

        let data_len = data.len();
        let arc_data = Arc::new(data);

        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut bytes = self.current_bytes.lock().unwrap_or_else(|e| e.into_inner());

        // 如果 key 已存在，先移除旧值的大小
        if let Some(old) = cache.pop(&key) {
            *bytes = bytes.saturating_sub(old.len());
        }

        // 超字节预算时按 LRU 驱逐旧条目
        while *bytes + data_len > self.max_bytes {
            match cache.pop_lru() {
                Some((_, evicted)) => {
                    *bytes = bytes.saturating_sub(evicted.len());
                }
                None => break,
            }
        }

        cache.put(key, arc_data);
        *bytes += data_len;
    }

    /// 从缓存中移除指定 key（用于解码后释放原始字节）。
    pub fn remove(&self, key: &str) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = cache.pop(key) {
            let mut bytes = self.current_bytes.lock().unwrap_or_else(|e| e.into_inner());
            *bytes = bytes.saturating_sub(old.len());
        }
    }
}
