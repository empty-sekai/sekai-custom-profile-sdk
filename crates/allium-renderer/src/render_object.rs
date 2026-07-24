//! Versioned mmap-backed premultiplied RGBA render objects.
//!
//! The store is deliberately independent from Skia. Offline builders decode
//! source images or compose stable component recipes once; the hot renderer
//! maps immutable pages and consumes row-major RGBA bytes directly.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use memmap2::{Mmap, MmapOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use allium_renderer_core::ResourceKey;

pub const RENDER_OBJECT_MANIFEST_SCHEMA: &str = "allium.render-object-manifest.v1";
pub const RENDER_OBJECT_GENERATOR_CONTRACT: &str =
    "allium.render-object.rgba8-premul-srgb-row-major.v1";
pub const RENDER_OBJECT_PIXEL_FORMAT: &str = "rgba8-premul-srgb";
pub const RENDER_OBJECT_PAGE_MAGIC: &[u8; 10] = b"ALLIUMRGBA";
pub const RENDER_OBJECT_PAGE_VERSION: u32 = 1;
pub const RENDER_OBJECT_PAGE_HEADER_BYTES: usize = 64;
pub const RENDER_OBJECT_ALIGNMENT: u64 = 64;
pub const PROFILE_RENDER_OBJECT_PREWARM_PREFIXES: &[&str] = &[
    "texture:assets/bonds_honor/",
    "texture:assets/character/member_cutout/",
    "texture:assets/chara_avatar/",
    "texture:assets/event_story/",
    "texture:assets/honor/",
    "texture:assets/thumbnail/chara/",
    "texture:assets/unit_story/",
    "texture:static/honor/",
    "component:general-base/",
    "component:deck-art-variant/",
];
const PREWARM_PAGE_STRIDE: usize = 4096;
const DECK_ART_VARIANT_PREWARM_CONTRACT: &str =
    "allium.deck-art-variant.sdk-6b0dae58.crop312x512.slot148x243.v1";
pub const HONOR_RENDER_OBJECT_CONTRACT: &str = "allium.honor-final.shared-core.v1";

pub fn standard_honor_object_key(honor_id: i32, honor_level: i32, full_size: bool) -> String {
    format!(
        "standard_honor:{HONOR_RENDER_OBJECT_CONTRACT}/id-{honor_id:08}/level-{honor_level:03}/{}",
        if full_size { "main" } else { "sub" }
    )
}

pub fn bonds_honor_object_key(
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_virtual_singer: bool,
) -> String {
    let word_id = if full_size { word_id } else { 0 };
    format!(
        "bonds_honor:{HONOR_RENDER_OBJECT_CONTRACT}/id-{honor_id:08}/level-{honor_level:03}/word-{word_id:08}/inverse-{}/vs-{}/{}",
        u8::from(inverse),
        u8::from(use_unit_virtual_singer),
        if full_size { "main" } else { "sub" }
    )
}

pub fn render_object_key_for_resource(resource: &ResourceKey) -> String {
    if resource.namespace == "render-object" {
        resource.key.clone()
    } else {
        format!("texture:{}/{}", resource.namespace, resource.key)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderObjectKind {
    Texture,
    StandardHonor,
    BondsHonor,
    CardMember,
    Component,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderObjectManifest {
    pub schema: String,
    pub generator_contract: String,
    pub pixel_format: String,
    /// Identity of the game asset/masterdata/raster recipe input set.
    pub source_identity: String,
    pub pages: Vec<RenderObjectPageManifest>,
    pub objects: Vec<RenderObjectEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderObjectPageManifest {
    pub file: String,
    pub payload_bytes: u64,
    pub object_count: u32,
    pub file_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderObjectEntry {
    /// Stable recipe identity used by compiled image/component commands.
    pub key: String,
    pub kind: RenderObjectKind,
    /// SHA-256 of the canonical source asset or canonical serialized recipe.
    pub source_sha256: String,
    pub page: u16,
    /// Byte offset from the beginning of the page payload, not the file.
    pub offset: u64,
    pub length: u64,
    pub width: u32,
    pub height: u32,
    pub row_bytes: u32,
    /// SHA-256 of exactly `length` premultiplied RGBA bytes.
    pub pixel_sha256: String,
}

#[derive(Debug, Error)]
pub enum RenderObjectError {
    #[error("render-object I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("render-object JSON failed for {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid render-object contract: {0}")]
    Invalid(String),
    #[error("render-object page hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("render-object pixel hash mismatch for {key}: expected {expected}, got {actual}")]
    PixelHashMismatch {
        key: String,
        expected: String,
        actual: String,
    },
}

pub struct MappedRenderObjectPage {
    payload_bytes: usize,
    mapping: Mmap,
}

impl MappedRenderObjectPage {
    pub fn payload(&self) -> &[u8] {
        &self.mapping
            [RENDER_OBJECT_PAGE_HEADER_BYTES..RENDER_OBJECT_PAGE_HEADER_BYTES + self.payload_bytes]
    }

    pub fn mapped_bytes(&self) -> usize {
        self.mapping.len()
    }
}

#[derive(Clone, Copy)]
pub struct MappedRenderObject<'a> {
    pub entry: &'a RenderObjectEntry,
    pub pixels: &'a [u8],
}

/// Borrowed catalog metadata for semantic resource resolution. This view does
/// not touch the mapped pixel payload and therefore cannot trigger image
/// decoding or page faults beyond the already-open manifest.
#[derive(Clone, Copy, Debug)]
pub struct RenderObjectMetadata<'a> {
    pub key: &'a str,
    pub kind: RenderObjectKind,
    pub width: u32,
    pub height: u32,
    pub row_bytes: u32,
    pub pixel_format: &'a str,
    pub source_identity: &'a str,
    pub source_sha256: &'a str,
    pub pixel_sha256: &'a str,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderObjectPrewarmReport {
    pub object_count: u64,
    pub object_bytes: u64,
    pub page_touch_count: u64,
    pub checksum: u64,
}

impl MappedRenderObject<'_> {
    pub fn row(&self, y: u32) -> Option<&[u8]> {
        if y >= self.entry.height {
            return None;
        }
        let start = usize::try_from(y)
            .ok()?
            .checked_mul(usize::try_from(self.entry.row_bytes).ok()?)?;
        let visible = usize::try_from(self.entry.width).ok()?.checked_mul(4)?;
        self.pixels.get(start..start.checked_add(visible)?)
    }
}

pub struct MappedRenderObjectStore {
    manifest_path: PathBuf,
    manifest: RenderObjectManifest,
    manifest_sha256: String,
    pages: Vec<MappedRenderObjectPage>,
    object_by_key: BTreeMap<String, usize>,
    resource_by_namespace: BTreeMap<String, BTreeMap<String, usize>>,
}

/// Atomically publishes immutable render-object generations. A request must
/// call `current()` once and retain that `Arc` for its complete render.
pub struct RenderObjectGenerationManager {
    current: ArcSwap<MappedRenderObjectStore>,
    previous: Mutex<Option<Arc<MappedRenderObjectStore>>>,
    publish_lock: Mutex<()>,
}

impl RenderObjectGenerationManager {
    pub fn open(manifest_path: impl AsRef<Path>) -> Result<Self, RenderObjectError> {
        Ok(Self::new(Arc::new(MappedRenderObjectStore::open(
            manifest_path,
        )?)))
    }

    pub fn new(current: Arc<MappedRenderObjectStore>) -> Self {
        Self {
            current: ArcSwap::from(current),
            previous: Mutex::new(None),
            publish_lock: Mutex::new(()),
        }
    }

    pub fn current(&self) -> Arc<MappedRenderObjectStore> {
        self.current.load_full()
    }

    pub fn previous(&self) -> Option<Arc<MappedRenderObjectStore>> {
        self.previous
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn publish_manifest(
        &self,
        manifest_path: impl AsRef<Path>,
    ) -> Result<Arc<MappedRenderObjectStore>, RenderObjectError> {
        let next = Arc::new(MappedRenderObjectStore::open(manifest_path)?);
        self.publish_prepared(next)
    }

    /// Publishes a generation that has already been opened, verified and
    /// optionally prewarmed by a background worker.
    pub fn publish_prepared(
        &self,
        next: Arc<MappedRenderObjectStore>,
    ) -> Result<Arc<MappedRenderObjectStore>, RenderObjectError> {
        let _publish = self
            .publish_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let old = self.current.load_full();
        self.current.store(Arc::clone(&next));
        *self
            .previous
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(old);
        Ok(next)
    }

    /// Atomically swaps back to the retained previous generation. Existing
    /// requests keep whichever `Arc` they pinned before the rollback.
    pub fn rollback(&self) -> Option<Arc<MappedRenderObjectStore>> {
        let _publish = self
            .publish_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut previous = self
            .previous
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let rollback = previous.take()?;
        let old = self.current.load_full();
        self.current.store(Arc::clone(&rollback));
        *previous = Some(old);
        Some(rollback)
    }
}

pub struct RenderObjectStoreWriter {
    output_dir: PathBuf,
    source_identity: String,
    page_payload_limit: u64,
    pages: Vec<RenderObjectPageManifest>,
    objects: Vec<RenderObjectEntry>,
    current_page: Option<WritablePage>,
    last_key: Option<String>,
}

struct WritablePage {
    index: u16,
    path: PathBuf,
    file: File,
    payload_bytes: u64,
    object_count: u32,
}

pub struct RenderObjectWrite<'a> {
    pub key: &'a str,
    pub kind: RenderObjectKind,
    pub source_sha256: &'a str,
    pub width: u32,
    pub height: u32,
    pub row_bytes: u32,
    pub pixels: &'a [u8],
}

pub fn hardlink_merge_stores(
    base_manifest_path: impl AsRef<Path>,
    delta_manifest_path: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
    source_identity: impl Into<String>,
) -> Result<PathBuf, RenderObjectError> {
    let base_manifest_path = base_manifest_path.as_ref();
    let delta_manifest_path = delta_manifest_path.as_ref();
    let output_dir = output_dir.as_ref();
    let source_identity = source_identity.into();
    let base = MappedRenderObjectStore::open_metadata_catalog(base_manifest_path)?;
    let delta = MappedRenderObjectStore::open_metadata_catalog(delta_manifest_path)?;
    if source_identity.is_empty() {
        return Err(RenderObjectError::Invalid(
            "merged render-object source identity is empty".into(),
        ));
    }
    if (
        base.manifest.schema.as_str(),
        base.manifest.generator_contract.as_str(),
        base.manifest.pixel_format.as_str(),
    ) != (
        delta.manifest.schema.as_str(),
        delta.manifest.generator_contract.as_str(),
        delta.manifest.pixel_format.as_str(),
    ) {
        return Err(RenderObjectError::Invalid(
            "base and delta render-object contracts differ".into(),
        ));
    }
    if output_dir.exists() {
        return Err(RenderObjectError::Invalid(format!(
            "merged render-object output already exists: {}",
            output_dir.display()
        )));
    }
    fs::create_dir(output_dir).map_err(|source| RenderObjectError::Io {
        path: output_dir.to_path_buf(),
        source,
    })?;

    let total_pages = base
        .manifest
        .pages
        .len()
        .checked_add(delta.manifest.pages.len())
        .ok_or_else(|| RenderObjectError::Invalid("merged page count overflow".into()))?;
    if total_pages > usize::from(u16::MAX) + 1 {
        return Err(RenderObjectError::Invalid(
            "merged render-object store has too many pages".into(),
        ));
    }
    let delta_page_offset = u16::try_from(base.manifest.pages.len()).map_err(|_| {
        RenderObjectError::Invalid("base render-object store has too many pages".into())
    })?;
    let mut pages = Vec::with_capacity(total_pages);
    for (manifest_path, manifest) in [
        (base_manifest_path, &base.manifest),
        (delta_manifest_path, &delta.manifest),
    ] {
        let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        for page in &manifest.pages {
            let file = format!("page-{:04}.rgba", pages.len());
            let source_path = root.join(&page.file);
            let destination = output_dir.join(&file);
            fs::hard_link(&source_path, &destination).map_err(|source| RenderObjectError::Io {
                path: destination,
                source,
            })?;
            let mut page = page.clone();
            page.file = file;
            pages.push(page);
        }
    }

    let mut objects = base.manifest.objects.clone();
    let mut keys = objects
        .iter()
        .map(|object| object.key.clone())
        .collect::<BTreeSet<_>>();
    for object in &delta.manifest.objects {
        if !keys.insert(object.key.clone()) {
            return Err(RenderObjectError::Invalid(format!(
                "delta render-object store duplicates base key {}",
                object.key
            )));
        }
        let mut object = object.clone();
        object.page = object.page.checked_add(delta_page_offset).ok_or_else(|| {
            RenderObjectError::Invalid(format!("page index overflow for {}", object.key))
        })?;
        objects.push(object);
    }
    objects.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    let manifest = RenderObjectManifest {
        schema: base.manifest.schema.clone(),
        generator_contract: base.manifest.generator_contract.clone(),
        pixel_format: base.manifest.pixel_format.clone(),
        source_identity,
        pages,
        objects,
    };
    validate_manifest(&manifest)?;
    let manifest_path = output_dir.join("manifest.json");
    let temporary = output_dir.join(".manifest.json.tmp");
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|source| RenderObjectError::Json {
        path: manifest_path.clone(),
        source,
    })?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|source| RenderObjectError::Io {
            path: temporary.clone(),
            source,
        })?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|source| RenderObjectError::Io {
            path: temporary.clone(),
            source,
        })?;
    drop(file);
    fs::rename(&temporary, &manifest_path).map_err(|source| RenderObjectError::Io {
        path: manifest_path.clone(),
        source,
    })?;
    MappedRenderObjectStore::open(&manifest_path)?;
    Ok(manifest_path)
}

impl MappedRenderObjectStore {
    pub fn open(manifest_path: impl AsRef<Path>) -> Result<Self, RenderObjectError> {
        let manifest_path = manifest_path.as_ref();
        let (bytes, manifest) = read_manifest(manifest_path)?;
        let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let pages = manifest
            .pages
            .iter()
            .map(|page| map_page(root, page))
            .collect::<Result<Vec<_>, _>>()?;
        let store = Self::from_manifest(manifest_path.to_path_buf(), bytes, manifest, pages);
        store.verify_pixels()?;
        Ok(store)
    }

    /// Opens only the immutable manifest catalog. This validates the complete
    /// manifest contract but deliberately does not open, mmap, hash, or touch
    /// pixel page files. It is intended for non-rendering resource preflight;
    /// [`Self::object`] consequently returns `None` for catalog-only stores.
    pub fn open_metadata_catalog(
        manifest_path: impl AsRef<Path>,
    ) -> Result<Self, RenderObjectError> {
        let (bytes, manifest) = read_manifest(manifest_path.as_ref())?;
        Ok(Self::from_manifest(
            manifest_path.as_ref().to_path_buf(),
            bytes,
            manifest,
            Vec::new(),
        ))
    }

    fn from_manifest(
        manifest_path: PathBuf,
        bytes: Vec<u8>,
        manifest: RenderObjectManifest,
        pages: Vec<MappedRenderObjectPage>,
    ) -> Self {
        let object_by_key = manifest
            .objects
            .iter()
            .enumerate()
            .map(|(index, object)| (object.key.clone(), index))
            .collect();
        let mut resource_by_namespace = BTreeMap::<String, BTreeMap<String, usize>>::new();
        for (index, object) in manifest.objects.iter().enumerate() {
            let Some(resource) = object.key.strip_prefix("texture:") else {
                continue;
            };
            let Some((namespace, key)) = resource.split_once('/') else {
                continue;
            };
            resource_by_namespace
                .entry(namespace.to_owned())
                .or_default()
                .insert(key.to_owned(), index);
        }
        Self {
            manifest_path,
            manifest,
            manifest_sha256: hex::encode(Sha256::digest(&bytes)),
            pages,
            object_by_key,
            resource_by_namespace,
        }
    }

    pub fn manifest(&self) -> &RenderObjectManifest {
        &self.manifest
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub fn mapped_bytes(&self) -> u64 {
        self.pages
            .iter()
            .map(|page| page.mapped_bytes() as u64)
            .fold(0, u64::saturating_add)
    }

    /// Looks up immutable manifest metadata by the semantic resource identity
    /// used by renderer-core. No render-object key formatting or pixel access
    /// occurs on this hot path.
    pub fn resource_metadata(
        &self,
        namespace: &str,
        key: &str,
    ) -> Option<RenderObjectMetadata<'_>> {
        let index = *self.resource_by_namespace.get(namespace)?.get(key)?;
        self.metadata_at(index)
    }

    /// Looks up immutable metadata by the canonical render-object key.
    pub fn metadata(&self, key: &str) -> Option<RenderObjectMetadata<'_>> {
        self.metadata_at(*self.object_by_key.get(key)?)
    }

    pub fn object(&self, key: &str) -> Option<MappedRenderObject<'_>> {
        let entry = &self.manifest.objects[*self.object_by_key.get(key)?];
        let page = self.pages.get(usize::from(entry.page))?;
        let start = usize::try_from(entry.offset).ok()?;
        let end = start.checked_add(usize::try_from(entry.length).ok()?)?;
        Some(MappedRenderObject {
            entry,
            pixels: page.payload().get(start..end)?,
        })
    }

    /// Ask the kernel to page in one immutable object without synchronously
    /// faulting every 4 KiB page on the request thread.
    #[cfg(unix)]
    pub fn advise_object_will_need(&self, key: &str) -> Option<std::io::Result<()>> {
        let entry = &self.manifest.objects[*self.object_by_key.get(key)?];
        let page = self.pages.get(usize::from(entry.page))?;
        let offset =
            RENDER_OBJECT_PAGE_HEADER_BYTES.checked_add(usize::try_from(entry.offset).ok()?)?;
        let length = usize::try_from(entry.length).ok()?;
        Some(
            page.mapping
                .advise_range(memmap2::Advice::WillNeed, offset, length),
        )
    }

    /// Synchronously faults the process-global profile hotset before a worker
    /// announces READY. The checksum makes the reads observable and provides
    /// a compact diagnostic without retaining or copying pixel buffers.
    pub fn prewarm_profile_hotset(&self) -> RenderObjectPrewarmReport {
        let mut report = RenderObjectPrewarmReport::default();
        let deck_sources = self.manifest.objects.iter().filter(|entry| {
            entry
                .key
                .starts_with("texture:assets/character/member_cutout/")
        });
        let mut deck_source_count = 0usize;
        let mut use_deck_variants = true;
        for entry in deck_sources {
            deck_source_count = deck_source_count.saturating_add(1);
            let mut digest = Sha256::new();
            digest.update(DECK_ART_VARIANT_PREWARM_CONTRACT.as_bytes());
            digest.update(entry.pixel_sha256.as_bytes());
            digest.update(entry.width.to_le_bytes());
            digest.update(entry.height.to_le_bytes());
            let key = format!(
                "component:deck-art-variant/{DECK_ART_VARIANT_PREWARM_CONTRACT}/{}",
                hex::encode(digest.finalize())
            );
            if !self.object_by_key.contains_key(&key) {
                use_deck_variants = false;
                break;
            }
        }
        use_deck_variants &= deck_source_count != 0;
        for entry in &self.manifest.objects {
            let is_deck_source = entry
                .key
                .starts_with("texture:assets/character/member_cutout/");
            if (is_deck_source && use_deck_variants)
                || !PROFILE_RENDER_OBJECT_PREWARM_PREFIXES
                    .iter()
                    .any(|prefix| entry.key.starts_with(prefix))
            {
                continue;
            }
            let Some(page) = self.pages.get(usize::from(entry.page)) else {
                continue;
            };
            let Some(start) = usize::try_from(entry.offset).ok() else {
                continue;
            };
            let Some(length) = usize::try_from(entry.length).ok() else {
                continue;
            };
            let Some(end) = start.checked_add(length) else {
                continue;
            };
            let Some(pixels) = page.payload().get(start..end) else {
                continue;
            };
            report.object_count = report.object_count.saturating_add(1);
            report.object_bytes = report.object_bytes.saturating_add(entry.length);
            for value in pixels.iter().step_by(PREWARM_PAGE_STRIDE) {
                report.checksum = report.checksum.rotate_left(1) ^ u64::from(*value);
                report.page_touch_count = report.page_touch_count.saturating_add(1);
            }
            if let Some(value) = pixels.last() {
                report.checksum = report.checksum.rotate_left(1) ^ u64::from(*value);
            }
        }
        std::hint::black_box(report)
    }

    fn metadata_at(&self, index: usize) -> Option<RenderObjectMetadata<'_>> {
        let entry = self.manifest.objects.get(index)?;
        Some(RenderObjectMetadata {
            key: &entry.key,
            kind: entry.kind,
            width: entry.width,
            height: entry.height,
            row_bytes: entry.row_bytes,
            pixel_format: &self.manifest.pixel_format,
            source_identity: &self.manifest.source_identity,
            source_sha256: &entry.source_sha256,
            pixel_sha256: &entry.pixel_sha256,
        })
    }

    fn verify_pixels(&self) -> Result<(), RenderObjectError> {
        for entry in &self.manifest.objects {
            let object = self.object(&entry.key).ok_or_else(|| {
                RenderObjectError::Invalid(format!(
                    "object {} is outside its mapped page",
                    entry.key
                ))
            })?;
            let actual = hex::encode(Sha256::digest(object.pixels));
            if actual != entry.pixel_sha256 {
                return Err(RenderObjectError::PixelHashMismatch {
                    key: entry.key.clone(),
                    expected: entry.pixel_sha256.clone(),
                    actual,
                });
            }
        }
        Ok(())
    }
}

impl RenderObjectStoreWriter {
    pub fn create(
        output_dir: impl AsRef<Path>,
        source_identity: impl Into<String>,
        page_payload_limit: u64,
    ) -> Result<Self, RenderObjectError> {
        let output_dir = output_dir.as_ref().to_path_buf();
        let source_identity = source_identity.into();
        if source_identity.trim().is_empty() {
            return Err(RenderObjectError::Invalid(
                "source identity must not be empty".into(),
            ));
        }
        if page_payload_limit < RENDER_OBJECT_ALIGNMENT {
            return Err(RenderObjectError::Invalid(format!(
                "page payload limit must be at least {RENDER_OBJECT_ALIGNMENT} bytes"
            )));
        }
        if output_dir.exists() {
            let mut entries =
                std::fs::read_dir(&output_dir).map_err(|source| RenderObjectError::Io {
                    path: output_dir.clone(),
                    source,
                })?;
            if entries.next().is_some() {
                return Err(RenderObjectError::Invalid(format!(
                    "output directory {} is not empty",
                    output_dir.display()
                )));
            }
        } else {
            std::fs::create_dir_all(&output_dir).map_err(|source| RenderObjectError::Io {
                path: output_dir.clone(),
                source,
            })?;
        }
        Ok(Self {
            output_dir,
            source_identity,
            page_payload_limit,
            pages: Vec::new(),
            objects: Vec::new(),
            current_page: None,
            last_key: None,
        })
    }

    /// Add one canonical object. Callers must provide strictly increasing keys
    /// so generation is deterministic without retaining all pixel buffers.
    pub fn add(&mut self, object: RenderObjectWrite<'_>) -> Result<(), RenderObjectError> {
        if object.key.trim().is_empty()
            || !valid_sha256(object.source_sha256)
            || object.width == 0
            || object.height == 0
            || object.row_bytes < object.width.saturating_mul(4)
            || object.row_bytes % 4 != 0
        {
            return Err(RenderObjectError::Invalid(format!(
                "invalid object input {}",
                object.key
            )));
        }
        if self
            .last_key
            .as_deref()
            .is_some_and(|last| last >= object.key)
        {
            return Err(RenderObjectError::Invalid(format!(
                "object keys must be unique and strictly increasing: {}",
                object.key
            )));
        }
        let length = u64::from(object.row_bytes)
            .checked_mul(u64::from(object.height))
            .ok_or_else(|| RenderObjectError::Invalid("object length overflow".into()))?;
        if usize::try_from(length).ok() != Some(object.pixels.len()) {
            return Err(RenderObjectError::Invalid(format!(
                "pixel length does not match row_bytes * height for {}",
                object.key
            )));
        }

        self.ensure_page()?;
        let mut offset = align_up(
            self.current_page
                .as_ref()
                .map(|page| page.payload_bytes)
                .unwrap_or_default(),
            RENDER_OBJECT_ALIGNMENT,
        )?;
        let required = offset
            .checked_add(length)
            .ok_or_else(|| RenderObjectError::Invalid("object range overflow".into()))?;
        let current_has_objects = self
            .current_page
            .as_ref()
            .is_some_and(|page| page.object_count > 0);
        if current_has_objects && required > self.page_payload_limit {
            self.finish_page()?;
            self.ensure_page()?;
            offset = 0;
        }

        let page = self.current_page.as_mut().ok_or_else(|| {
            RenderObjectError::Invalid("render-object writer has no active page".into())
        })?;
        if offset > page.payload_bytes {
            write_zeros(&mut page.file, offset - page.payload_bytes, &page.path)?;
        }
        page.file
            .write_all(object.pixels)
            .map_err(|source| RenderObjectError::Io {
                path: page.path.clone(),
                source,
            })?;
        page.payload_bytes = offset.saturating_add(length);
        page.object_count = page.object_count.saturating_add(1);
        self.objects.push(RenderObjectEntry {
            key: object.key.to_string(),
            kind: object.kind,
            source_sha256: object.source_sha256.to_string(),
            page: page.index,
            offset,
            length,
            width: object.width,
            height: object.height,
            row_bytes: object.row_bytes,
            pixel_sha256: hex::encode(Sha256::digest(object.pixels)),
        });
        self.last_key = Some(object.key.to_string());
        Ok(())
    }

    pub fn finish(mut self) -> Result<PathBuf, RenderObjectError> {
        self.finish_page()?;
        if self.objects.is_empty() {
            return Err(RenderObjectError::Invalid(
                "render-object store must contain at least one object".into(),
            ));
        }
        let manifest = RenderObjectManifest {
            schema: RENDER_OBJECT_MANIFEST_SCHEMA.into(),
            generator_contract: RENDER_OBJECT_GENERATOR_CONTRACT.into(),
            pixel_format: RENDER_OBJECT_PIXEL_FORMAT.into(),
            source_identity: self.source_identity,
            pages: self.pages,
            objects: self.objects,
        };
        validate_manifest(&manifest)?;
        let manifest_path = self.output_dir.join("manifest.json");
        let bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|source| RenderObjectError::Json {
                path: manifest_path.clone(),
                source,
            })?;
        std::fs::write(&manifest_path, bytes).map_err(|source| RenderObjectError::Io {
            path: manifest_path.clone(),
            source,
        })?;
        Ok(manifest_path)
    }

    fn ensure_page(&mut self) -> Result<(), RenderObjectError> {
        if self.current_page.is_some() {
            return Ok(());
        }
        let index = u16::try_from(self.pages.len()).map_err(|_| {
            RenderObjectError::Invalid("render-object page count exceeds u16".into())
        })?;
        let path = self.output_dir.join(format!("page-{index:04}.rgba"));
        let mut file = File::create(&path).map_err(|source| RenderObjectError::Io {
            path: path.clone(),
            source,
        })?;
        file.write_all(&[0u8; RENDER_OBJECT_PAGE_HEADER_BYTES])
            .map_err(|source| RenderObjectError::Io {
                path: path.clone(),
                source,
            })?;
        self.current_page = Some(WritablePage {
            index,
            path,
            file,
            payload_bytes: 0,
            object_count: 0,
        });
        Ok(())
    }

    fn finish_page(&mut self) -> Result<(), RenderObjectError> {
        let Some(mut page) = self.current_page.take() else {
            return Ok(());
        };
        if page.object_count == 0 {
            return Err(RenderObjectError::Invalid(
                "cannot finalize an empty render-object page".into(),
            ));
        }
        let mut header = [0u8; RENDER_OBJECT_PAGE_HEADER_BYTES];
        header[..RENDER_OBJECT_PAGE_MAGIC.len()].copy_from_slice(RENDER_OBJECT_PAGE_MAGIC);
        header[12..16].copy_from_slice(&RENDER_OBJECT_PAGE_VERSION.to_le_bytes());
        header[16..24].copy_from_slice(&page.payload_bytes.to_le_bytes());
        header[24..28].copy_from_slice(&page.object_count.to_le_bytes());
        page.file
            .seek(SeekFrom::Start(0))
            .and_then(|_| page.file.write_all(&header))
            .and_then(|_| page.file.flush())
            .map_err(|source| RenderObjectError::Io {
                path: page.path.clone(),
                source,
            })?;
        drop(page.file);
        let file_sha256 = hash_file(&page.path)?;
        self.pages.push(RenderObjectPageManifest {
            file: page
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| RenderObjectError::Invalid("non-UTF8 page filename".into()))?
                .to_string(),
            payload_bytes: page.payload_bytes,
            object_count: page.object_count,
            file_sha256,
        });
        Ok(())
    }
}

fn read_manifest(
    manifest_path: &Path,
) -> Result<(Vec<u8>, RenderObjectManifest), RenderObjectError> {
    let bytes = std::fs::read(manifest_path).map_err(|source| RenderObjectError::Io {
        path: manifest_path.to_path_buf(),
        source,
    })?;
    let manifest = serde_json::from_slice(&bytes).map_err(|source| RenderObjectError::Json {
        path: manifest_path.to_path_buf(),
        source,
    })?;
    validate_manifest(&manifest)?;
    Ok((bytes, manifest))
}

fn validate_manifest(manifest: &RenderObjectManifest) -> Result<(), RenderObjectError> {
    if manifest.schema != RENDER_OBJECT_MANIFEST_SCHEMA
        || manifest.generator_contract != RENDER_OBJECT_GENERATOR_CONTRACT
        || manifest.pixel_format != RENDER_OBJECT_PIXEL_FORMAT
        || manifest.source_identity.trim().is_empty()
        || manifest.pages.is_empty()
    {
        return Err(RenderObjectError::Invalid(
            "unsupported schema/generator/pixel format, empty identity, or empty pages".into(),
        ));
    }

    for page in &manifest.pages {
        let path = Path::new(&page.file);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            || page.payload_bytes == 0
            || !valid_sha256(&page.file_sha256)
        {
            return Err(RenderObjectError::Invalid(format!(
                "invalid page descriptor {}",
                page.file
            )));
        }
    }

    let mut seen = BTreeSet::new();
    let mut intervals = vec![Vec::<(u64, u64, &str)>::new(); manifest.pages.len()];
    let mut page_counts = vec![0u32; manifest.pages.len()];
    for object in &manifest.objects {
        let page_index = usize::from(object.page);
        let expected_length = u64::from(object.row_bytes)
            .checked_mul(u64::from(object.height))
            .ok_or_else(|| RenderObjectError::Invalid("object length overflow".into()))?;
        let end = object.offset.checked_add(object.length).ok_or_else(|| {
            RenderObjectError::Invalid(format!("object {} range overflow", object.key))
        })?;
        if object.key.trim().is_empty()
            || !seen.insert(object.key.as_str())
            || !valid_sha256(&object.source_sha256)
            || !valid_sha256(&object.pixel_sha256)
            || page_index >= manifest.pages.len()
            || object.offset % RENDER_OBJECT_ALIGNMENT != 0
            || object.width == 0
            || object.height == 0
            || object.row_bytes < object.width.saturating_mul(4)
            || object.row_bytes % 4 != 0
            || object.length != expected_length
            || end > manifest.pages[page_index].payload_bytes
        {
            return Err(RenderObjectError::Invalid(format!(
                "invalid object descriptor {}",
                object.key
            )));
        }
        intervals[page_index].push((object.offset, end, object.key.as_str()));
        page_counts[page_index] = page_counts[page_index].saturating_add(1);
    }

    for (page_index, page_intervals) in intervals.iter_mut().enumerate() {
        page_intervals.sort_unstable_by_key(|interval| interval.0);
        if page_intervals
            .windows(2)
            .any(|window| window[0].1 > window[1].0)
            // A generation may intentionally leave superseded payload ranges
            // unreferenced so unchanged pages can be hardlinked across
            // incremental publications. The page header records physical
            // objects written to that immutable page; the manifest is allowed
            // to reference a subset, but never more than the physical count.
            || page_counts[page_index] > manifest.pages[page_index].object_count
        {
            return Err(RenderObjectError::Invalid(format!(
                "overlapping objects or inconsistent object_count on page {page_index}"
            )));
        }
    }
    Ok(())
}

fn map_page(
    root: &Path,
    descriptor: &RenderObjectPageManifest,
) -> Result<MappedRenderObjectPage, RenderObjectError> {
    let path = root.join(&descriptor.file);
    let file = File::open(&path).map_err(|source| RenderObjectError::Io {
        path: path.clone(),
        source,
    })?;
    // SAFETY: render-object artifacts are immutable and mapped read-only for
    // the worker lifetime. Their length/header/hash are validated below.
    let mapping =
        unsafe { MmapOptions::new().map(&file) }.map_err(|source| RenderObjectError::Io {
            path: path.clone(),
            source,
        })?;
    let payload_bytes = usize::try_from(descriptor.payload_bytes)
        .map_err(|_| RenderObjectError::Invalid("page payload does not fit usize".into()))?;
    let expected_len = RENDER_OBJECT_PAGE_HEADER_BYTES
        .checked_add(payload_bytes)
        .ok_or_else(|| RenderObjectError::Invalid("page length overflow".into()))?;
    if mapping.len() != expected_len
        || mapping.get(..RENDER_OBJECT_PAGE_MAGIC.len()) != Some(RENDER_OBJECT_PAGE_MAGIC)
        || read_u32(&mapping, 12) != Some(RENDER_OBJECT_PAGE_VERSION)
        || read_u64(&mapping, 16) != Some(descriptor.payload_bytes)
        || read_u32(&mapping, 24) != Some(descriptor.object_count)
    {
        return Err(RenderObjectError::Invalid(format!(
            "invalid render-object page header {}",
            path.display()
        )));
    }
    let actual = hex::encode(Sha256::digest(&mapping));
    if actual != descriptor.file_sha256 {
        return Err(RenderObjectError::HashMismatch {
            path,
            expected: descriptor.file_sha256.clone(),
            actual,
        });
    }
    Ok(MappedRenderObjectPage {
        payload_bytes,
        mapping,
    })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && hex::decode(value).is_ok()
}

fn align_up(value: u64, alignment: u64) -> Result<u64, RenderObjectError> {
    value
        .checked_add(alignment - 1)
        .map(|value| value / alignment * alignment)
        .ok_or_else(|| RenderObjectError::Invalid("alignment overflow".into()))
}

fn write_zeros(file: &mut File, mut count: u64, path: &Path) -> Result<(), RenderObjectError> {
    const ZEROS: [u8; 4096] = [0; 4096];
    while count > 0 {
        let chunk = usize::try_from(count.min(ZEROS.len() as u64))
            .map_err(|_| RenderObjectError::Invalid("padding length overflow".into()))?;
        file.write_all(&ZEROS[..chunk])
            .map_err(|source| RenderObjectError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        count -= chunk as u64;
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, RenderObjectError> {
    let mut file = File::open(path).map_err(|source| RenderObjectError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| RenderObjectError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset.checked_add(4)?)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    bytes
        .get(offset..offset.checked_add(8)?)?
        .try_into()
        .ok()
        .map(u64::from_le_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honor_keys_are_canonical_and_sub_ignores_word() {
        assert_eq!(
            standard_honor_object_key(42, 3, true),
            "standard_honor:allium.honor-final.shared-core.v1/id-00000042/level-003/main"
        );
        assert_eq!(
            bonds_honor_object_key(7, 10, false, 1234, true, false),
            bonds_honor_object_key(7, 10, false, 0, true, false)
        );
    }

    fn write_generation(root: &Path, identity: &str, value: u8) -> PathBuf {
        let pixels = vec![value; 2 * 2 * 4];
        let source_sha256 = hex::encode(Sha256::digest([value]));
        let mut writer = RenderObjectStoreWriter::create(root, identity, 1024).unwrap();
        writer
            .add(RenderObjectWrite {
                key: "texture:assets/generation-fixture",
                kind: RenderObjectKind::Texture,
                source_sha256: &source_sha256,
                width: 2,
                height: 2,
                row_bytes: 8,
                pixels: &pixels,
            })
            .unwrap();
        writer.finish().unwrap()
    }

    #[test]
    fn generation_publish_keeps_pinned_request_and_previous_alive() {
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        let first_manifest = write_generation(first_root.path(), "generation-1", 1);
        let second_manifest = write_generation(second_root.path(), "generation-2", 2);
        let manager = RenderObjectGenerationManager::open(first_manifest).unwrap();
        let pinned_request = manager.current();

        let published = manager.publish_manifest(second_manifest).unwrap();
        assert_eq!(published.manifest().source_identity, "generation-2");
        assert_eq!(manager.current().manifest().source_identity, "generation-2");
        assert_eq!(
            manager.previous().unwrap().manifest().source_identity,
            "generation-1"
        );
        assert_eq!(pinned_request.manifest().source_identity, "generation-1");
        assert_eq!(
            pinned_request
                .object("texture:assets/generation-fixture")
                .unwrap()
                .pixels[0],
            1
        );

        let rolled_back = manager.rollback().expect("previous generation");
        assert_eq!(rolled_back.manifest().source_identity, "generation-1");
        assert_eq!(manager.current().manifest().source_identity, "generation-1");
        assert_eq!(
            manager.previous().unwrap().manifest().source_identity,
            "generation-2"
        );
    }

    fn write_fixture(root: &Path) -> PathBuf {
        let pixels = [
            1u8, 2, 3, 4, 5, 6, 7, 8, // row 0
            9, 10, 11, 12, 13, 14, 15, 16, // row 1
        ];
        let mut page = vec![0u8; RENDER_OBJECT_PAGE_HEADER_BYTES + pixels.len()];
        page[..RENDER_OBJECT_PAGE_MAGIC.len()].copy_from_slice(RENDER_OBJECT_PAGE_MAGIC);
        page[12..16].copy_from_slice(&RENDER_OBJECT_PAGE_VERSION.to_le_bytes());
        page[16..24].copy_from_slice(&(pixels.len() as u64).to_le_bytes());
        page[24..28].copy_from_slice(&1u32.to_le_bytes());
        page[RENDER_OBJECT_PAGE_HEADER_BYTES..].copy_from_slice(&pixels);
        std::fs::write(root.join("objects.rgba"), &page).expect("write page");

        let manifest = RenderObjectManifest {
            schema: RENDER_OBJECT_MANIFEST_SCHEMA.into(),
            generator_contract: RENDER_OBJECT_GENERATOR_CONTRACT.into(),
            pixel_format: RENDER_OBJECT_PIXEL_FORMAT.into(),
            source_identity: "fixture-v1".into(),
            pages: vec![RenderObjectPageManifest {
                file: "objects.rgba".into(),
                payload_bytes: pixels.len() as u64,
                object_count: 1,
                file_sha256: hex::encode(Sha256::digest(&page)),
            }],
            objects: vec![RenderObjectEntry {
                key: "texture:fixture".into(),
                kind: RenderObjectKind::Texture,
                source_sha256: hex::encode(Sha256::digest(b"source")),
                page: 0,
                offset: 0,
                length: pixels.len() as u64,
                width: 2,
                height: 2,
                row_bytes: 8,
                pixel_sha256: hex::encode(Sha256::digest(pixels)),
            }],
        };
        let manifest_path = root.join("manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        manifest_path
    }

    #[test]
    fn opens_and_returns_mapped_rows() {
        let root = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_fixture(root.path());
        let store = MappedRenderObjectStore::open(manifest_path).expect("open store");
        let object = store.object("texture:fixture").expect("mapped object");
        assert_eq!(object.pixels.len(), 16);
        assert_eq!(object.row(0), Some(&[1, 2, 3, 4, 5, 6, 7, 8][..]));
        assert_eq!(object.row(1), Some(&[9, 10, 11, 12, 13, 14, 15, 16][..]));
        assert_eq!(object.row(2), None);
        assert_eq!(store.manifest().objects.len(), 1);
        assert_eq!(store.mapped_bytes(), 80);
    }

    #[test]
    fn metadata_catalog_does_not_open_pixel_pages() {
        let root = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_fixture(root.path());
        std::fs::remove_file(root.path().join("objects.rgba")).expect("remove pixel page");

        let store = MappedRenderObjectStore::open_metadata_catalog(&manifest_path)
            .expect("open metadata catalog");
        let metadata = store.metadata("texture:fixture").expect("object metadata");
        assert_eq!((metadata.width, metadata.height), (2, 2));
        assert_eq!(store.mapped_bytes(), 0);
        assert!(store.object("texture:fixture").is_none());
    }

    #[test]
    fn rejects_path_traversal_and_overlapping_objects() {
        let root = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_fixture(root.path());
        let mut manifest: RenderObjectManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("read manifest"))
                .expect("parse manifest");
        manifest.pages[0].file = "../objects.rgba".into();
        assert!(matches!(
            validate_manifest(&manifest),
            Err(RenderObjectError::Invalid(_))
        ));

        manifest.pages[0].file = "objects.rgba".into();
        manifest.pages[0].object_count = 2;
        let mut duplicate = manifest.objects[0].clone();
        duplicate.key = "texture:overlap".into();
        manifest.objects.push(duplicate);
        assert!(matches!(
            validate_manifest(&manifest),
            Err(RenderObjectError::Invalid(_))
        ));
    }

    #[test]
    fn accepts_unreferenced_physical_objects_for_incremental_hardlinks() {
        let root = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_fixture(root.path());
        let mut manifest: RenderObjectManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("read manifest"))
                .expect("parse manifest");
        manifest.objects.clear();
        validate_manifest(&manifest).expect("orphaned immutable payload is valid");
    }

    #[test]
    fn rejects_tampered_pixels_even_with_valid_page_hash() {
        let root = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_fixture(root.path());
        let mut page = std::fs::read(root.path().join("objects.rgba")).expect("read page");
        page[RENDER_OBJECT_PAGE_HEADER_BYTES] ^= 0xff;
        std::fs::write(root.path().join("objects.rgba"), &page).expect("write page");

        let mut manifest: RenderObjectManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("read manifest"))
                .expect("parse manifest");
        manifest.pages[0].file_sha256 = hex::encode(Sha256::digest(&page));
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        assert!(matches!(
            MappedRenderObjectStore::open(manifest_path),
            Err(RenderObjectError::PixelHashMismatch { .. })
        ));
    }

    #[test]
    fn writer_streams_aligned_pages_and_reopens_them() {
        let root = tempfile::tempdir().expect("tempdir");
        let output = root.path().join("store");
        let source = hex::encode(Sha256::digest(b"source"));
        let first = [1u8; 16];
        let second = [2u8; 16];
        let mut writer =
            RenderObjectStoreWriter::create(&output, "fixture-writer-v1", 64).expect("writer");
        writer
            .add(RenderObjectWrite {
                key: "texture:assets/a",
                kind: RenderObjectKind::Texture,
                source_sha256: &source,
                width: 2,
                height: 2,
                row_bytes: 8,
                pixels: &first,
            })
            .expect("add first");
        writer
            .add(RenderObjectWrite {
                key: "texture:assets/b",
                kind: RenderObjectKind::Texture,
                source_sha256: &source,
                width: 2,
                height: 2,
                row_bytes: 8,
                pixels: &second,
            })
            .expect("add second");
        let manifest_path = writer.finish().expect("finish store");
        let store = MappedRenderObjectStore::open(manifest_path).expect("reopen store");
        assert_eq!(store.manifest().pages.len(), 2);
        assert_eq!(store.object("texture:assets/a").expect("a").pixels, first);
        assert_eq!(store.object("texture:assets/b").expect("b").pixels, second);
        let metadata = store
            .resource_metadata("assets", "a")
            .expect("resource metadata");
        assert_eq!(
            (metadata.width, metadata.height, metadata.row_bytes),
            (2, 2, 8)
        );
        assert_eq!(metadata.pixel_format, RENDER_OBJECT_PIXEL_FORMAT);
        assert_eq!(metadata.source_identity, "fixture-writer-v1");
        assert_eq!(store.manifest().objects[0].offset, 0);
        assert_eq!(store.manifest().objects[1].offset, 0);
    }

    #[test]
    fn profile_hotset_prewarm_touches_only_declared_global_prefixes() {
        let root = tempfile::tempdir().expect("tempdir");
        let output = root.path().join("store");
        let source = hex::encode(Sha256::digest(b"source"));
        let hot = [3u8; 16];
        let cold = [7u8; 16];
        let mut writer =
            RenderObjectStoreWriter::create(&output, "prewarm-fixture", 128).expect("writer");
        for key in [
            "texture:assets/bonds_honor/example",
            "texture:assets/chara_avatar/chara01_02",
            "texture:assets/character/member_cutout/res/after_training",
            "texture:assets/event_story/example/screen_image/banner_event_story",
            "texture:assets/honor/example/degree_main",
            "texture:assets/thumbnail/chara/example",
            "texture:assets/unit_story/example/screen_image/banner_unit_story",
            "texture:static/honor/frame_degree_m_1",
        ] {
            writer
                .add(RenderObjectWrite {
                    key,
                    kind: RenderObjectKind::Texture,
                    source_sha256: &source,
                    width: 2,
                    height: 2,
                    row_bytes: 8,
                    pixels: &hot,
                })
                .expect("General hotset object");
        }
        writer
            .add(RenderObjectWrite {
                key: "texture:unrelated/example",
                kind: RenderObjectKind::Texture,
                source_sha256: &source,
                width: 2,
                height: 2,
                row_bytes: 8,
                pixels: &cold,
            })
            .expect("cold object");
        let manifest = writer.finish().expect("finish store");
        let store = MappedRenderObjectStore::open(manifest).expect("open store");
        let report = store.prewarm_profile_hotset();
        assert_eq!(report.object_count, 8);
        assert_eq!(report.object_bytes, 128);
        assert_eq!(report.page_touch_count, 8);
        assert_eq!(report.checksum, 65_537);
    }
}
