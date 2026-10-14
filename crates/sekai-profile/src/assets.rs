//! Asset storage for rendering: a cache of encoded bytes.
//!
//! [`AssetStore`] keeps the encoded bytes (PNG) of assets in one LRU cache with
//! a single byte budget, evicting the least recently used entries when the
//! budget is exceeded. Static assets go to a pinned pool outside the budget.
//!
//! Pixels are decoded on demand by `crate::codec` and premultiplied as
//! `round(value * alpha / 255)` (see [`crate::codec::premultiply_channel`]).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use lru::LruCache;

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
    /// The asset is not cached.
    Missing,
    /// The asset is cached but its pixels could not be read.
    Unreadable,
}

/// Decodes a shape sprite's identity from its PNG bytes: its size and the
/// hash of its stored red and alpha samples, as the shape atlas records it.
fn shape_sdf_source_identity_from_png(
    encoded: &[u8],
) -> Result<ShapeSdfSourceIdentity, ShapeSourceIdentityError> {
    if !crate::codec::png::is_png(encoded) {
        return Err(ShapeSourceIdentityError::Missing);
    }
    let source = crate::sdf::shape_atlas::ShapeSdfSource::from_png(encoded)
        .map_err(|_| ShapeSourceIdentityError::Missing)?;
    Ok(ShapeSdfSourceIdentity {
        width: i32::try_from(source.width).map_err(|_| ShapeSourceIdentityError::Unreadable)?,
        height: i32::try_from(source.height).map_err(|_| ShapeSourceIdentityError::Unreadable)?,
        rg8_sha256: source.rg8_sha256,
    })
}

/// Maps a cache key to a disk file name (`/` → `__`).
///
/// Most keys contain no `/` (such as `honor` or `stamp_123`) and are borrowed
/// as is; only keys with `/` (such as `honor/bonds/17`) allocate a new string.
fn normalize_disk_key(key: &str) -> std::borrow::Cow<'_, str> {
    if key.contains('/') {
        std::borrow::Cow::Owned(key.replace('/', "__"))
    } else {
        std::borrow::Cow::Borrowed(key)
    }
}

/// LRU cache of encoded bytes under one byte budget.
struct ByteLru {
    cache: LruCache<String, Arc<Vec<u8>>>,
    current_bytes: usize,
    max_bytes: usize,
    disk_cache_dir: Option<PathBuf>,
}

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

    /// Removes an entry from the LRU and releases its share of the budget.
    fn pop(&mut self, key: &str) -> Option<Arc<Vec<u8>>> {
        let data = self.cache.pop(key)?;
        self.current_bytes = self.current_bytes.saturating_sub(data.len());
        Some(data)
    }
}

/// Asset storage for rendering.
///
/// Keeps encoded asset bytes, keyed by asset key (for example
/// `honor/frame_degree_m_1`), and decodes pixels on demand. Two tiers hold the
/// bytes:
///
/// - an LRU cache under one byte budget, optionally backed by a disk cache
///   directory ([`AssetStore::set_disk_cache_dir`]);
/// - a pinned pool of static assets ([`AssetStore::load_static_dir`]) that
///   takes no part in the budget and is never evicted.
///
/// Apart from [`AssetStore::set_disk_cache_dir`], the methods take `&self`, so
/// one store can be shared behind an `Arc` by a renderer and its callers.
pub struct AssetStore {
    /// Encoded bytes, evicted by the byte budget.
    cache: Mutex<ByteLru>,
    /// Keys pinned as static assets, so resource resolution can consult them.
    pinned_static_keys: Mutex<BTreeSet<String>>,
    /// Pinned encoded bytes of static assets, outside the LRU and the budget.
    pinned_bytes: Mutex<HashMap<String, Arc<Vec<u8>>>>,
    shape_sdf_identities: Mutex<HashMap<String, ShapeSdfSourceIdentity>>,
}

impl AssetStore {
    /// Creates an empty store.
    ///
    /// `max_mb` is the byte budget of the LRU cache in MiB, shared by all
    /// cached assets. A budget larger than `usize` bytes is treated as
    /// unlimited.
    pub fn new(max_mb: usize) -> Self {
        Self {
            cache: Mutex::new(ByteLru::new(max_mb.saturating_mul(1024 * 1024))),
            pinned_static_keys: Mutex::new(BTreeSet::new()),
            pinned_bytes: Mutex::new(HashMap::new()),
            shape_sdf_identities: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the asset as premultiplied RGBA8, tightly packed.
    ///
    /// The store keeps encoded bytes, so this decodes through the engine's own
    /// codec and premultiplies each channel as `round(value * alpha / 255)`.
    /// Non-PNG payloads are refused rather than guessed at.
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
        for pixel in pixels.as_chunks_mut::<4>().0 {
            let alpha = pixel[3];
            for value in &mut pixel[..3] {
                *value = crate::codec::premultiply_channel(*value, alpha);
            }
        }
        Some((decoded.width, decoded.height, pixels))
    }

    /// Sets a disk cache directory, created if missing. Every later
    /// [`AssetStore::put`] also writes the bytes there, and a cache miss falls
    /// back to reading the file, so downloaded assets survive a restart.
    ///
    /// The directory is the scope of the cache. A hit looks up the file named
    /// after the key and does not check a content version, so the caller must
    /// keep the key-to-bytes mapping stable within one directory: when the
    /// upstream content of a key changes, use another directory or clear this
    /// one before setting it. [`AssetStore::contains`] counts disk hits as
    /// present, so a caller that uses it to decide whether to fetch will not
    /// refetch a stale file.
    pub fn set_disk_cache_dir(&mut self, dir: std::path::PathBuf) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_disk_cache_dir(dir);
    }

    /// Whether `key` is in the pinned pool, the LRU cache or the disk cache
    /// directory.
    pub fn contains(&self, key: &str) -> bool {
        if self
            .pinned_bytes
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

    /// Whether `key` was pinned as a static asset by
    /// [`AssetStore::load_static_dir`].
    pub fn is_pinned_static(&self, key: &str) -> bool {
        self.pinned_static_keys
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(key)
    }

    /// Puts an asset's encoded bytes into the LRU cache, and into the disk
    /// cache directory when one is set.
    ///
    /// Earlier content under the same key is replaced. Whether the bytes decode
    /// is checked when pixels are read.
    pub fn put(&self, key: String, data: Vec<u8>) {
        self.shape_sdf_identities
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&key);
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put(key, data);
    }

    /// Returns the encoded bytes of `key`, or `None` when the store does not
    /// hold it.
    ///
    /// Lookup order: pinned pool, then LRU cache, then the disk cache
    /// directory.
    pub fn get(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        if let Some(data) = self
            .pinned_bytes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
        {
            return Some(Arc::clone(data));
        }
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
    }

    /// Pixel size of an image asset, read from its PNG header under the same
    /// format rules the decoder applies. It is the size the image draws at when
    /// nothing else sizes it.
    pub fn image_size(&self, key: &str) -> Option<(u32, u32)> {
        let encoded = self.get(key)?;
        crate::codec::png::dimensions(&encoded).ok()
    }

    /// Puts every PNG under `dir` into the cache, keyed by its path relative
    /// to `dir` without the extension: `honor/frame_degree_m_1.png` becomes
    /// `honor/frame_degree_m_1`. Returns the number of files read.
    ///
    /// Only PNG files are read, the one format the renderer decodes. Images
    /// beyond the byte budget evict earlier ones as any other put does, so
    /// size the store to hold the directory.
    ///
    /// # Errors
    ///
    /// Returns a message naming the path when a directory or file cannot be
    /// read.
    pub fn load_png_dir(&self, dir: &std::path::Path) -> Result<usize, String> {
        let files = read_png_dir(dir)?;
        let count = files.len();
        for (key, data) in files {
            self.put(key, data);
        }
        Ok(count)
    }

    /// Loads every PNG under `dir` into the static pool, keyed as
    /// [`AssetStore::load_png_dir`] keys them. Static assets take no part in
    /// the byte budget, are never evicted, and are reported by
    /// [`AssetStore::is_pinned_static`]. Returns the number of files read; a
    /// file whose PNG header the decoder rejects is skipped with a warning.
    ///
    /// # Errors
    ///
    /// Returns a message naming the path when a directory or file cannot be
    /// read.
    pub fn load_static_dir(&self, dir: &std::path::Path) -> Result<usize, String> {
        let files = read_png_dir(dir)?;
        let count = files.len();
        let pinned = self.pin_static(files);
        tracing::info!(loaded = count, pinned, "static assets loaded");
        Ok(count)
    }

    /// Resolves the source identity for a shape asset key: the size and hash
    /// of the stored red and alpha samples, the same stream the shape atlas
    /// builder copies and records.
    pub(crate) fn shape_sdf_source_identity_for_key(
        &self,
        key: &str,
    ) -> Result<ShapeSdfSourceIdentity, ShapeSourceIdentityError> {
        if let Some(identity) = self
            .shape_sdf_identities
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
        {
            return Ok(identity.clone());
        }
        let encoded = self.get(key).ok_or(ShapeSourceIdentityError::Missing)?;
        let identity = shape_sdf_source_identity_from_png(&encoded)?;
        self.shape_sdf_identities
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(key.to_string(), identity.clone());
        Ok(identity)
    }

    /// Moves loaded static assets into the pinned pool and returns how many
    /// were pinned.
    ///
    /// The encoded bytes are pinned once their PNG header passes the decoder's
    /// checks. Pinned assets take no part in the LRU budget and are never
    /// evicted.
    fn pin_static(&self, keys_and_data: Vec<(String, Vec<u8>)>) -> usize {
        let mut count = 0usize;
        let mut pinned = self.pinned_bytes.lock().unwrap_or_else(|e| e.into_inner());
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        for (key, data) in keys_and_data {
            if let Err(error) = crate::codec::png::dimensions(&data) {
                tracing::warn!(
                    asset_key = key,
                    bytes = data.len(),
                    %error,
                    "static asset is not a PNG the decoder accepts; not pinning it"
                );
                continue;
            }
            cache.pop(&key);
            self.shape_sdf_identities
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&key);
            self.pinned_static_keys
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(key.clone());
            pinned.insert(key, Arc::new(data));
            count += 1;
        }
        count
    }
}

/// Reads every PNG under `base`, keyed by its relative path without the
/// extension, with `/` separators.
fn read_png_dir(base: &std::path::Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    fn walk(
        base: &std::path::Path,
        dir: &std::path::Path,
        files: &mut Vec<(String, Vec<u8>)>,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(dir)
            .map_err(|error| format!("read directory {} failed: {error}", dir.display()))?;
        for entry in entries {
            let path = entry
                .map_err(|error| format!("read directory {} failed: {error}", dir.display()))?
                .path();
            if path.is_dir() {
                walk(base, &path, files)?;
                continue;
            }
            let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            if !ext.eq_ignore_ascii_case("png") {
                continue;
            }
            let relative = path
                .strip_prefix(base)
                .map_err(|error| {
                    format!("{} is outside {}: {error}", path.display(), base.display())
                })?
                .to_string_lossy()
                .replace('\\', "/");
            // strip_suffix strips exactly once and matches the original
            // casing: `.PNG` is stripped too, and `foo.png.png` keeps one
            // `.png`.
            let key = relative
                .strip_suffix(&format!(".{ext}"))
                .unwrap_or(&relative)
                .to_string();
            let data = std::fs::read(&path)
                .map_err(|error| format!("read {} failed: {error}", path.display()))?;
            files.push((key, data));
        }
        Ok(())
    }

    let mut files = Vec::new();
    walk(base, base, &mut files)?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn shape_source_identity_hashes_the_stored_red_and_alpha_samples() {
        let pixels = [
            200, 10, 20, 3, 17, 0, 0, 128, 255, 255, 255, 255, 90, 1, 2, 0,
        ];
        let encoded = crate::codec::png::encode_rgba(2, 2, &pixels).expect("encode probe");
        let store = AssetStore::new(4);
        store.put("custom_profile/shape/probe".into(), encoded);
        let identity = store
            .shape_sdf_source_identity_for_key("custom_profile/shape/probe")
            .expect("probe identity");
        let mut hasher = Sha256::new();
        for pixel in pixels.as_chunks::<4>().0 {
            hasher.update([pixel[0], pixel[3]]);
        }
        assert_eq!((identity.width, identity.height), (2, 2));
        assert_eq!(identity.rg8_sha256, hex::encode(hasher.finalize()));
        assert_eq!(
            store.shape_sdf_source_identity_for_key("custom_profile/shape/absent"),
            Err(ShapeSourceIdentityError::Missing)
        );
    }

    #[test]
    fn static_directory_images_are_stored_pinned_and_sized() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("honor")).expect("static subdirectory");
        let encoded = crate::codec::png::encode_rgba(3, 2, &[255u8; 24]).expect("encode probe");
        std::fs::write(dir.path().join("honor/frame_degree_m_1.png"), &encoded)
            .expect("write probe");
        // A budget of zero keeps at most the latest cached entry, so a pinned
        // static image has to survive later stores.
        let store = AssetStore::new(0);
        assert_eq!(store.load_static_dir(dir.path()), Ok(1));
        store.put(
            "other".into(),
            crate::codec::png::encode_rgba(1, 1, &[0, 0, 0, 255]).expect("encode other"),
        );
        let key = "honor/frame_degree_m_1";
        assert!(store.contains(key));
        assert!(store.is_pinned_static(key));
        assert_eq!(store.image_size(key), Some((3, 2)));
        assert_eq!(
            store.get_premultiplied_rgba(key).map(|(w, h, _)| (w, h)),
            Some((3, 2))
        );
    }

    #[test]
    fn budgets_too_large_to_count_in_bytes_saturate() {
        let store = AssetStore::new(usize::MAX);
        let encoded = crate::codec::png::encode_rgba(1, 1, &[1, 2, 3, 255]).expect("encode");
        store.put("probe".into(), encoded);
        assert_eq!(store.image_size("probe"), Some((1, 1)));
    }

    #[test]
    fn storing_undecodable_bytes_replaces_the_previous_image() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = AssetStore::new(4);
        store.set_disk_cache_dir(dir.path().to_path_buf());
        let encoded = crate::codec::png::encode_rgba(1, 1, &[1, 2, 3, 255]).expect("encode");
        store.put("probe".into(), encoded);
        assert_eq!(
            store.get_premultiplied_rgba("probe"),
            Some((1, 1, vec![1, 2, 3, 255]))
        );
        store.put("probe".into(), b"not an image".to_vec());
        assert_eq!(store.image_size("probe"), None);
        assert_eq!(store.get_premultiplied_rgba("probe"), None);
        // The replacement reached the disk cache too: a store reopened over the
        // same directory reads the new bytes, not the old image.
        let mut reopened = AssetStore::new(4);
        reopened.set_disk_cache_dir(dir.path().to_path_buf());
        assert_eq!(reopened.image_size("probe"), None);
    }

    #[test]
    fn a_png_directory_loads_by_relative_key() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("honor")).expect("subdirectory");
        let encoded = crate::codec::png::encode_rgba(1, 1, &[1, 2, 3, 255]).expect("encode");
        std::fs::write(dir.path().join("honor/a.png"), &encoded).expect("write png");
        std::fs::write(dir.path().join("b.jpg"), b"not read").expect("write jpg");
        let store = AssetStore::new(4);
        assert_eq!(store.load_png_dir(dir.path()), Ok(1));
        assert_eq!(store.image_size("honor/a"), Some((1, 1)));
        assert!(!store.contains("b"));
        assert!(!store.is_pinned_static("honor/a"));
    }
}
