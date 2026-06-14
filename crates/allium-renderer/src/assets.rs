//! 渲染素材存储（统一 Image 缓存）
//!
//! `AssetStore` 使用单一 LRU 缓存管理解码后的 Skia Image。
//! 素材下载后立即解码为 Image 存入缓存，不保留原始字节。
//! 统一字节预算，超限按 LRU 驱逐。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lru::LruCache;

#[cfg(feature = "skia-core")]
use std::collections::HashMap;

/// 将缓存 key 规范化为磁盘文件名（`/` → `__`）。
///
/// 多数 key 不含 `/`（如 `honor`、`stamp_123`），此时直接借用原串零分配；
/// 仅含 `/` 的 key（如 `honor/bonds/17`）才分配新 String。
fn normalize_disk_key(key: &str) -> std::borrow::Cow<'_, str> {
    if key.contains('/') {
        std::borrow::Cow::Owned(key.replace('/', "__"))
    } else {
        std::borrow::Cow::Borrowed(key)
    }
}

/// 统一字节预算的 LRU 缓存，只存解码后的 Image。
#[cfg(feature = "skia-core")]
struct ImageLru {
    cache: LruCache<String, skia_safe::Image>,
    current_bytes: usize,
    max_bytes: usize,
    /// 磁盘缓存目录（可选，原始字节持久化，重启后重新解码加载）
    disk_cache_dir: Option<PathBuf>,
}

#[cfg(feature = "skia-core")]
impl ImageLru {
    fn new(max_bytes: usize) -> Self {
        Self {
            cache: LruCache::unbounded(),
            current_bytes: 0,
            max_bytes,
            disk_cache_dir: None,
        }
    }

    fn set_disk_cache_dir(&mut self, dir: PathBuf) {
        std::fs::create_dir_all(&dir).ok();
        self.disk_cache_dir = Some(dir);
    }

    fn image_bytes(img: &skia_safe::Image) -> usize {
        (img.width() as usize)
            .saturating_mul(img.height() as usize)
            .saturating_mul(4)
    }

    fn get(&mut self, key: &str) -> Option<skia_safe::Image> {
        self.cache.get(key).cloned()
    }

    /// 存入解码后的 Image，超预算时 LRU 驱逐。
    fn put(&mut self, key: String, img: skia_safe::Image) {
        let new_bytes = Self::image_bytes(&img);

        if new_bytes > self.max_bytes {
            self.cache.clear();
            self.cache.put(key, img);
            self.current_bytes = new_bytes;
            return;
        }

        // 如果 key 已存在，先移除旧值
        if let Some(old) = self.cache.pop(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(Self::image_bytes(&old));
        }

        // 超字节预算时驱逐
        while self.current_bytes + new_bytes > self.max_bytes {
            match self.cache.pop_lru() {
                Some((_, evicted)) => {
                    self.current_bytes = self.current_bytes.saturating_sub(Self::image_bytes(&evicted));
                }
                None => break,
            }
        }

        self.cache.put(key, img);
        self.current_bytes += new_bytes;
    }

    /// 检查 key 是否在缓存或磁盘中。
    fn contains(&self, key: &str) -> bool {
        self.cache.contains(key)
            || self
                .disk_cache_dir
                .as_ref()
                .is_some_and(|dir| dir.join(&*normalize_disk_key(key)).exists())
    }

    /// 将条目移至常驻池（释放字节预算）。
    fn pop(&mut self, key: &str) -> Option<skia_safe::Image> {
        if let Some(img) = self.cache.pop(key) {
            self.current_bytes = self.current_bytes.saturating_sub(Self::image_bytes(&img));
            return Some(img);
        }
        None
    }

    /// 从磁盘加载原始字节并解码存入缓存。
    fn load_from_disk(&mut self, key: &str) -> bool {
        if let Some(ref dir) = self.disk_cache_dir {
            let path = dir.join(&*normalize_disk_key(key));
            if let Ok(data) = std::fs::read(&path) {
                let skia_data = skia_safe::Data::new_copy(&data);
                if let Some(image) = skia_safe::Image::from_encoded(skia_data) {
                    self.put(key.to_string(), image);
                    return true;
                }
            }
        }
        false
    }

    /// 持久化原始字节到磁盘。
    fn write_to_disk(&self, key: &str, data: &[u8]) {
        if let Some(ref dir) = self.disk_cache_dir {
            let path = dir.join(&*normalize_disk_key(key));
            std::fs::write(&path, data).ok();
        }
    }
}

/// 非 skia 构建的占位缓存。
#[cfg(not(feature = "skia-core"))]
struct ByteLru {
    cache: LruCache<String, Arc<Vec<u8>>>,
    current_bytes: usize,
    max_bytes: usize,
    disk_cache_dir: Option<PathBuf>,
}

#[cfg(not(feature = "skia-core"))]
impl ByteLru {
    fn new(max_bytes: usize) -> Self {
        Self {
            cache: LruCache::unbounded(),
            current_bytes: 0,
            max_bytes,
            disk_cache_dir: None,
        }
    }

    fn set_disk_cache_dir(&mut self, dir: PathBuf) {
        std::fs::create_dir_all(&dir).ok();
        self.disk_cache_dir = Some(dir);
    }

    fn contains(&self, key: &str) -> bool {
        self.cache.contains(key)
            || self
                .disk_cache_dir
                .as_ref()
                .is_some_and(|dir| dir.join(&*normalize_disk_key(key)).exists())
    }

    fn get(&mut self, key: &str) -> Option<Arc<Vec<u8>>> {
        if let Some(v) = self.cache.get(key).cloned() {
            return Some(v);
        }
        if let Some(ref dir) = self.disk_cache_dir {
            let path = dir.join(&*normalize_disk_key(key));
            if let Ok(data) = std::fs::read(&path) {
                let data_len = data.len();
                let arc = Arc::new(data);
                while self.current_bytes + data_len > self.max_bytes {
                    match self.cache.pop_lru() {
                        Some((_, evicted)) => {
                            self.current_bytes = self.current_bytes.saturating_sub(evicted.len());
                        }
                        None => break,
                    }
                }
                self.cache.put(key.to_string(), arc.clone());
                self.current_bytes += data_len;
                return Some(arc);
            }
        }
        None
    }

    fn put(&mut self, key: String, data: Vec<u8>) {
        if let Some(ref dir) = self.disk_cache_dir {
            let path = dir.join(&*normalize_disk_key(&key));
            std::fs::write(&path, &data).ok();
        }
        let data_len = data.len();
        let arc_data = Arc::new(data);
        if let Some(old) = self.cache.pop(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(old.len());
        }
        while self.current_bytes + data_len > self.max_bytes {
            match self.cache.pop_lru() {
                Some((_, evicted)) => {
                    self.current_bytes = self.current_bytes.saturating_sub(evicted.len());
                }
                None => break,
            }
        }
        self.cache.put(key, arc_data);
        self.current_bytes += data_len;
    }
}

/// 渲染用素材存储。
///
/// 单一 LRU 缓存管理解码后的 Image，统一字节预算。
/// 下载后立即解码，不保留原始字节（磁盘缓存独立持久化）。
/// 常驻池（pinned）不受预算约束。
pub struct AssetStore {
    /// 解码 Image 缓存（字节预算驱动驱逐）
    #[cfg(feature = "skia-core")]
    cache: Mutex<ImageLru>,
    /// 原始字节缓存（非 skia 构建降级）
    #[cfg(not(feature = "skia-core"))]
    cache: Mutex<ByteLru>,
    /// 静态素材常驻池（不走 LRU，启动时预解码，不占预算）
    #[cfg(feature = "skia-core")]
    pinned_images: Mutex<HashMap<String, skia_safe::Image>>,
}

impl AssetStore {
    /// 创建素材存储
    ///
    /// `max_mb` 为总缓存预算（MB），所有 Image 共享此额度。
    #[cfg(feature = "skia-core")]
    pub fn new(max_mb: usize) -> Self {
        Self {
            cache: Mutex::new(ImageLru::new(max_mb * 1024 * 1024)),
            pinned_images: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(not(feature = "skia-core"))]
    pub fn new(max_mb: usize) -> Self {
        Self {
            cache: Mutex::new(ByteLru::new(max_mb * 1024 * 1024)),
        }
    }

    /// 设置磁盘缓存目录（S3 下载的资源持久化到磁盘，重启不丢失）
    pub fn set_disk_cache_dir(&mut self, dir: std::path::PathBuf) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).set_disk_cache_dir(dir);
    }

    /// 检查 key 是否存在于缓存或磁盘中。
    pub fn contains(&self, key: &str) -> bool {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).contains(key)
    }

    /// 将素材放入缓存（立即解码为 Image，原始字节写磁盘）。
    #[cfg(feature = "skia-core")]
    pub fn put(&self, key: String, data: Vec<u8>) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        // 先写磁盘（持久化原始字节供重启后重新加载）
        cache.write_to_disk(&key, &data);
        // 立即解码
        let skia_data = skia_safe::Data::new_copy(&data);
        if let Some(image) = skia_safe::Image::from_encoded(skia_data) {
            cache.put(key, image);
        }
        // data 在此处 drop，不保留原始字节
    }

    /// 将素材放入缓存（非 skia 构建降级：只存原始字节）。
    #[cfg(not(feature = "skia-core"))]
    pub fn put(&self, key: String, data: Vec<u8>) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).put(key, data);
    }

    /// 从缓存获取原始字节（仅非 skia 构建使用）。
    #[cfg(not(feature = "skia-core"))]
    pub fn get(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(key)
    }

    /// 从静态素材目录加载打包资源并预解码到常驻池。
    pub fn load_static_dir(&self, dir: &std::path::Path) -> Result<usize, String> {
        let mut count = 0usize;
        let mut keys_and_data: Vec<(String, Vec<u8>)> = Vec::new();

        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("读取目录 {} 失败: {e}", dir.display()))?;
        Self::walk_static_dir_recursive(dir, entries, &mut count, &mut keys_and_data)?;

        #[cfg(feature = "skia-core")]
        {
            let decoded = self.pre_decode_static(&keys_and_data);
            tracing::info!(loaded = count, decoded, "静态素材预解码完成");
        }
        Ok(count)
    }

    fn walk_static_dir_recursive(
        base: &std::path::Path,
        entries: std::fs::ReadDir,
        count: &mut usize,
        keys_and_data: &mut Vec<(String, Vec<u8>)>,
    ) -> Result<(), String> {
        for entry in entries {
            let entry = entry.map_err(|e| format!("遍历目录失败: {e}"))?;
            let path = entry.path();
            if path.is_dir() {
                let sub_entries = std::fs::read_dir(&path)
                    .map_err(|e| format!("读取目录 {} 失败: {e}", path.display()))?;
                Self::walk_static_dir_recursive(base, sub_entries, count, keys_and_data)?;
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if ext_lower == "png" || ext_lower == "jpg" {
                    let rel = path
                        .strip_prefix(base)
                        .map_err(|e| format!("路径前缀错误: {e}"))?;
                    let key = rel.to_string_lossy().replace('\\', "/");
                    let key = key
                        .trim_end_matches(".png")
                        .trim_end_matches(".jpg")
                        .to_string();
                    let data = std::fs::read(&path)
                        .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
                    keys_and_data.push((key, data));
                    *count += 1;
                }
            }
        }
        Ok(())
    }

    // === Skia Image 解码（渲染专属） ===

    /// 获取解码后的 Skia Image。
    ///
    /// 查找顺序：常驻池 → LRU 缓存 → 磁盘回退（重新解码）。
    #[cfg(feature = "skia-core")]
    pub fn get_image(&self, key: &str) -> Option<skia_safe::Image> {
        // 1. 常驻池
        {
            let pinned = self.pinned_images.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(img) = pinned.get(key) {
                return Some(img.clone());
            }
        }

        // 2. LRU 缓存
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(img) = cache.get(key) {
            return Some(img);
        }

        // 3. 磁盘回退
        if cache.load_from_disk(key) {
            return cache.get(key);
        }

        None
    }

    /// 将已加载的静态素材预解码并移入常驻池。
    ///
    /// 调用后这些素材不占用 LRU 预算，永不被驱逐。
    #[cfg(feature = "skia-core")]
    fn pre_decode_static(&self, keys_and_data: &[(String, Vec<u8>)]) -> usize {
        let mut count = 0usize;
        let mut pinned = self.pinned_images.lock().unwrap_or_else(|e| e.into_inner());
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        for (key, data) in keys_and_data {
            let skia_data = skia_safe::Data::new_copy(data);
            if let Some(image) = skia_safe::Image::from_encoded(skia_data) {
                // 先尝试从 LRU 取出（如果 put 已经放进去）
                cache.pop(key);
                pinned.insert(key.clone(), image);
                count += 1;
            }
        }
        count
    }
}
