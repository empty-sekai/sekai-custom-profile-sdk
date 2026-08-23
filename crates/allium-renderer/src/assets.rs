//! 渲染素材存储（统一 Image 缓存）
//!
//! `AssetStore` 使用单一 LRU 缓存管理解码后的 Skia Image。
//! 素材下载后立即解码为 Image 存入缓存，不保留原始字节。
//! 统一字节预算，超限按 LRU 驱逐。
//!
//! Decoding is done by `crate::codec`, not by Skia. The decoded samples are
//! premultiplied here — `round(value * alpha / 255)`, which is bit-identical to
//! what a Skia decode produces internally — and wrapped as a raster image for the
//! draw path. `png-parity` gates that equivalence over the asset corpus and over
//! every (value, alpha) pair.

use std::path::PathBuf;
#[cfg(not(feature = "skia-core"))]
use std::sync::Arc;
use std::sync::Mutex;

use lru::LruCache;

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::collections::HashMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShapeSdfSourceIdentity {
    pub width: i32,
    pub height: i32,
    pub rg8_sha256: String,
}

/// Why a shape's decoded source identity could not be resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShapeSourceIdentityError {
    /// The asset is not cached (or, with a raster backend, failed to decode
    /// into the image cache).
    Missing,
    /// The asset is cached but its pixels could not be read.
    Unreadable,
}

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

/// Decodes an encoded asset into a premultiplied raster image.
///
/// Only PNG is accepted: it is the only container the profile asset pipeline
/// produces, and a decoder that guesses at other formats would risk handing the
/// compositor a wrong-coloured surface it cannot distinguish from authored
/// content. Anything else is refused with a logged reason rather than decoded
/// partially or replaced by a blank.
#[cfg(feature = "skia-core")]
fn decode_asset(key: &str, encoded: &[u8]) -> Option<skia_safe::Image> {
    if !crate::codec::png::is_png(encoded) {
        tracing::warn!(
            asset_key = key,
            bytes = encoded.len(),
            "asset is not a PNG; refusing to decode it"
        );
        return None;
    }
    let decoded = match crate::codec::png::decode(encoded) {
        Ok(decoded) => decoded,
        Err(error) => {
            tracing::warn!(asset_key = key, %error, "asset PNG could not be decoded");
            return None;
        }
    };
    let mut pixels = decoded.pixels;
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha < 255 {
            pixel[0] = crate::codec::premultiply_channel(pixel[0], alpha);
            pixel[1] = crate::codec::premultiply_channel(pixel[1], alpha);
            pixel[2] = crate::codec::premultiply_channel(pixel[2], alpha);
        }
    }
    let width = i32::try_from(decoded.width).ok()?;
    let height = i32::try_from(decoded.height).ok()?;
    let row_bytes = usize::try_from(decoded.width).ok()?.checked_mul(4)?;
    let info = skia_safe::ImageInfo::new(
        (width, height),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Premul,
        None,
    );
    let image =
        skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(&pixels), row_bytes);
    if image.is_none() {
        tracing::warn!(
            asset_key = key,
            width = decoded.width,
            height = decoded.height,
            "decoded asset could not be wrapped as a raster image"
        );
    }
    image
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
                    self.current_bytes = self
                        .current_bytes
                        .saturating_sub(Self::image_bytes(&evicted));
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
                if let Some(image) = decode_asset(key, &data) {
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
    /// Keys pinned as static assets. Independent of how (or whether) they were
    /// decoded, so resource resolution can consult it in any build.
    pinned_static_keys: Mutex<BTreeSet<String>>,
    /// 静态素材常驻池（不走 LRU，启动时预解码，不占预算）
    #[cfg(feature = "skia-core")]
    pinned_images: Mutex<HashMap<String, skia_safe::Image>>,
    shape_sdf_identities: Mutex<HashMap<String, ShapeSdfSourceIdentity>>,
    /// Missing image identities observed since the last audit drain. This is
    /// populated only on a real lookup miss and is therefore off the hit path.
    #[cfg(feature = "skia-core")]
    missing_image_keys: Mutex<BTreeSet<String>>,
}

impl AssetStore {
    /// Returns the asset as premultiplied RGBA8, tightly packed.
    ///
    /// The cached image already holds exactly this buffer, so the read is a copy
    /// rather than a conversion. Reading it back as non-premultiplied and
    /// multiplying by alpha again would produce the same bytes but lose precision
    /// on the way through.
    #[cfg(feature = "skia-core")]
    pub fn get_premultiplied_rgba(&self, key: &str) -> Option<(u32, u32, Vec<u8>)> {
        let image = self.get_image(key)?;
        let width = u32::try_from(image.width()).ok()?;
        let height = u32::try_from(image.height()).ok()?;
        let row_bytes = usize::try_from(width).ok()?.checked_mul(4)?;
        let mut pixels = vec![0u8; row_bytes.checked_mul(usize::try_from(height).ok()?)?];
        let info = skia_safe::ImageInfo::new(
            (image.width(), image.height()),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        if !image.read_pixels(
            &info,
            &mut pixels,
            row_bytes,
            (0, 0),
            skia_safe::image::CachingHint::Allow,
        ) {
            return None;
        }
        Some((width, height, pixels))
    }

    /// 创建素材存储
    ///
    /// `max_mb` 为总缓存预算（MB），所有 Image 共享此额度。
    #[cfg(feature = "skia-core")]
    pub fn new(max_mb: usize) -> Self {
        Self {
            cache: Mutex::new(ImageLru::new(max_mb * 1024 * 1024)),
            pinned_static_keys: Mutex::new(BTreeSet::new()),
            pinned_images: Mutex::new(HashMap::new()),
            shape_sdf_identities: Mutex::new(HashMap::new()),
            missing_image_keys: Mutex::new(BTreeSet::new()),
        }
    }

    #[cfg(not(feature = "skia-core"))]
    pub fn new(max_mb: usize) -> Self {
        Self {
            cache: Mutex::new(ByteLru::new(max_mb * 1024 * 1024)),
            pinned_static_keys: Mutex::new(BTreeSet::new()),
            shape_sdf_identities: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the asset as premultiplied RGBA8, tightly packed.
    ///
    /// Without a raster backend the store keeps encoded bytes, so this decodes
    /// through the engine's own codec and premultiplies with the same rounding
    /// the backend-backed path uses. Non-PNG payloads are refused rather than
    /// guessed at.
    #[cfg(not(feature = "skia-core"))]
    pub fn get_premultiplied_rgba(&self, key: &str) -> Option<(u32, u32, Vec<u8>)> {
        let encoded = self.get(key)?;
        if !crate::codec::png::is_png(&encoded) {
            tracing::warn!(
                asset_key = key,
                bytes = encoded.len(),
                "asset is not a PNG; refusing to decode it"
            );
            return None;
        }
        let decoded = crate::codec::png::decode(&encoded).ok()?;
        let mut pixels = decoded.pixels;
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = pixel[3];
            for channel in 0..3 {
                pixel[channel] = crate::codec::premultiply_channel(pixel[channel], alpha);
            }
        }
        Some((decoded.width, decoded.height, pixels))
    }

    /// 设置磁盘缓存目录（S3 下载的资源持久化到磁盘，重启不丢失）
    pub fn set_disk_cache_dir(&mut self, dir: std::path::PathBuf) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_disk_cache_dir(dir);
    }

    /// 检查 key 是否存在于缓存或磁盘中。
    pub fn contains(&self, key: &str) -> bool {
        #[cfg(feature = "skia-core")]
        if self
            .pinned_images
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(key)
        {
            return true;
        }
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(key)
    }

    pub fn is_pinned_static(&self, key: &str) -> bool {
        self.pinned_static_keys
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(key)
    }

    /// 将素材放入缓存（立即解码为 Image，原始字节写磁盘）。
    #[cfg(feature = "skia-core")]
    pub fn put(&self, key: String, data: Vec<u8>) {
        self.shape_sdf_identities
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&key);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        // 先写磁盘（持久化原始字节供重启后重新加载）
        cache.write_to_disk(&key, &data);
        // 立即解码
        if let Some(image) = decode_asset(&key, &data) {
            cache.put(key, image);
        }
        // data 在此处 drop，不保留原始字节
    }

    /// 将素材放入缓存（非 skia 构建降级：只存原始字节）。
    #[cfg(not(feature = "skia-core"))]
    pub fn put(&self, key: String, data: Vec<u8>) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put(key, data);
    }

    /// 从缓存获取原始字节（仅非 skia 构建使用）。
    #[cfg(not(feature = "skia-core"))]
    pub fn get(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
    }

    /// 从静态素材目录加载打包资源并预解码到常驻池。
    pub fn load_static_dir(&self, dir: &std::path::Path) -> Result<usize, String> {
        let mut count = 0usize;
        let mut keys_and_data: Vec<(String, Vec<u8>)> = Vec::new();

        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("读取目录 {} 失败: {e}", dir.display()))?;
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
        self.get_image_with_audit(key, true)
    }

    /// Looks up an intentionally optional recipe candidate without emitting a
    /// repair miss. Use this only when the recipe has an explicit fallback or
    /// omission contract for the key.
    #[cfg(feature = "skia-core")]
    pub fn get_image_optional(&self, key: &str) -> Option<skia_safe::Image> {
        self.get_image_with_audit(key, false)
    }

    #[cfg(feature = "skia-core")]
    fn get_image_with_audit(&self, key: &str, record_missing: bool) -> Option<skia_safe::Image> {
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

        drop(cache);
        if record_missing {
            self.missing_image_keys
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(key.to_owned());
        }
        None
    }

    /// Drains the deduplicated image misses observed by render recipes. The
    /// offline object builder uses this for structured repair input; request
    /// rendering continues to follow the recipe's existing optional/fallback
    /// behavior.
    #[cfg(feature = "skia-core")]
    pub fn take_missing_image_keys(&self) -> Vec<String> {
        let mut missing = self
            .missing_image_keys
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut *missing).into_iter().collect()
    }

    #[cfg(feature = "skia-core")]
    pub(crate) fn shape_sdf_source_identity(
        &self,
        key: &str,
        image: &skia_safe::Image,
    ) -> Option<ShapeSdfSourceIdentity> {
        let mut identities = self
            .shape_sdf_identities
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(identity) = identities.get(key) {
            return Some(identity.clone());
        }

        let width = image.width();
        let height = image.height();
        let width_usize = usize::try_from(width).ok()?;
        let height_usize = usize::try_from(height).ok()?;
        let row_bytes = width_usize.checked_mul(4)?;
        let mut rgba = vec![0u8; row_bytes.checked_mul(height_usize)?];
        let info = skia_safe::ImageInfo::new(
            (width, height),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        if !image.read_pixels(
            &info,
            &mut rgba,
            row_bytes,
            (0, 0),
            skia_safe::image::CachingHint::Allow,
        ) {
            return None;
        }
        let mut hasher = Sha256::new();
        for pixel in rgba.chunks_exact(4) {
            hasher.update([pixel[0], pixel[3]]);
        }
        let identity = ShapeSdfSourceIdentity {
            width,
            height,
            rg8_sha256: hex::encode(hasher.finalize()),
        };
        identities.insert(key.to_string(), identity.clone());
        Some(identity)
    }

    /// Resolves the decoded source identity for a shape asset key, whichever
    /// decode path this build carries. With a raster backend the identity is
    /// read from the cached image exactly as before; without one the cached
    /// PNG bytes decode through the engine's own codec, whose straight RGBA is
    /// the same stream the atlas builder hashes.
    pub(crate) fn shape_sdf_source_identity_for_key(
        &self,
        key: &str,
    ) -> Result<ShapeSdfSourceIdentity, ShapeSourceIdentityError> {
        #[cfg(feature = "skia-core")]
        {
            let image = self
                .get_image(key)
                .ok_or(ShapeSourceIdentityError::Missing)?;
            self.shape_sdf_source_identity(key, &image)
                .ok_or(ShapeSourceIdentityError::Unreadable)
        }
        #[cfg(not(feature = "skia-core"))]
        {
            if let Some(identity) = self
                .shape_sdf_identities
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(key)
            {
                return Ok(identity.clone());
            }
            let encoded = self.get(key).ok_or(ShapeSourceIdentityError::Missing)?;
            if !crate::codec::png::is_png(&encoded) {
                return Err(ShapeSourceIdentityError::Missing);
            }
            let decoded = crate::codec::png::decode(&encoded)
                .map_err(|_| ShapeSourceIdentityError::Missing)?;
            let width =
                i32::try_from(decoded.width).map_err(|_| ShapeSourceIdentityError::Unreadable)?;
            let height =
                i32::try_from(decoded.height).map_err(|_| ShapeSourceIdentityError::Unreadable)?;
            let mut hasher = Sha256::new();
            for pixel in decoded.pixels.chunks_exact(4) {
                hasher.update([pixel[0], pixel[3]]);
            }
            let identity = ShapeSdfSourceIdentity {
                width,
                height,
                rg8_sha256: hex::encode(hasher.finalize()),
            };
            self.shape_sdf_identities
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(key.to_string(), identity.clone());
            Ok(identity)
        }
    }

    /// 将已加载的静态素材预解码并移入常驻池。
    ///
    /// 调用后这些素材不占用 LRU 预算，永不被驱逐。
    #[cfg(feature = "skia-core")]
    fn pre_decode_static(&self, keys_and_data: &[(String, Vec<u8>)]) -> usize {
        let mut identities = self
            .shape_sdf_identities
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (key, _) in keys_and_data {
            identities.remove(key);
        }
        drop(identities);
        let mut count = 0usize;
        let mut pinned = self.pinned_images.lock().unwrap_or_else(|e| e.into_inner());
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        for (key, data) in keys_and_data {
            if let Some(image) = decode_asset(key, data) {
                // 先尝试从 LRU 取出（如果 put 已经放进去）
                cache.pop(key);
                pinned.insert(key.clone(), image);
                self.pinned_static_keys
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(key.clone());
                count += 1;
            }
        }
        count
    }
}
