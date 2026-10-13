//! Sparse, persistent fallback glyph cache.
//!
//! Only codepoints that miss their selected profile font are generated. Each
//! glyph is stored in its own minimal 8x8-aligned mmap page so adding one
//! fallback never requires generating the fallback font's complete cmap.
//!
//! One cache root serves one font. Its layout is
//!
//! ```text
//! <root>/cache.lock                     advisory lock serializing writers
//! <root>/current.json                   pointer to the published manifest
//! <root>/<identity>/pages/uXXXXXXXX.r8swz   one page per glyph, written once
//! <root>/<identity>/manifest-<gen>.json     the glyph set of one generation
//! ```
//!
//! `<identity>` digests the font bytes, the generator contract and the
//! sampling parameters, so a replaced font file or a changed generator starts
//! from an empty directory instead of reusing glyphs rendered from something
//! else. A new generation adds pages only for new codepoints and a new
//! manifest; publishing it removes the superseded manifests, pages no manifest
//! references and directories of other identities.
//!
//! A codepoint the font cannot supply is recorded in the manifest's
//! generation failures rather than failing the generation, and counts as
//! resolved from then on, so later requests do not generate it again.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ttf_parser::Face;

use super::atlas::{
    MappedSdfAtlas, SdfAtlasGenerationFailure, SdfAtlasGenerationReport, SdfAtlasGlyphManifest,
    SdfAtlasManifest, SdfAtlasPageManifest, ATLAS_MANIFEST_SCHEMA,
    PROFILE_TEXT_FALLBACK_FONT_FAMILY, SWIZZLED_BLOCK_HEIGHT, SWIZZLED_BLOCK_WIDTH,
    SWIZZLED_PAGE_HEADER_BYTES, SWIZZLED_PAGE_MAGIC, SWIZZLED_PAGE_VERSION,
};
use super::outline::{self, OfflineAtlasGlyphGenerator, OfflineGenerationMethod, OutlineSdfGlyph};

pub const SOURCE_HAN_SANS_SC_FAMILY: &str = PROFILE_TEXT_FALLBACK_FONT_FAMILY;
const POINTER_SCHEMA: &str = "allium.sdf-fallback-cache-pointer.v2";
const POINTER_FILE: &str = "current.json";
const LOCK_FILE: &str = "cache.lock";
const PAGES_DIR: &str = "pages";
const MANIFEST_PREFIX: &str = "manifest-";
/// Generation directory of the layout written by earlier releases; its
/// content is superseded by the identity directories and reclaimed on publish.
const SUPERSEDED_GENERATIONS_DIR: &str = "generations";
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(30);
const FALLBACK_SUPERSAMPLE: usize = 2;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistentFallbackSdfCacheReport {
    pub requested_codepoint_count: u64,
    /// Requested codepoints already resolved, as a glyph or as a codepoint
    /// the font cannot supply.
    pub cache_hit_count: u64,
    /// Requested codepoints that got a glyph in this call.
    pub generated_codepoint_count: u64,
    /// Codepoints resolved after this call, glyphs and unsupported alike.
    pub total_cached_codepoint_count: u64,
    pub generated_file_bytes: u64,
    pub font_family: String,
    pub font_sha256: String,
    pub manifest_sha256: Option<String>,
    pub elapsed_ns: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CachePointer {
    schema: String,
    font_family: String,
    font_sha256: String,
    generator_contract: String,
    manifest: String,
}

/// The generation this process currently serves.
#[derive(Clone)]
struct Generation {
    /// Manifest path relative to the cache root, as the pointer records it.
    manifest: String,
    atlas: Arc<MappedSdfAtlas>,
}

impl Generation {
    /// Codepoints the generation resolved: its glyphs and the codepoints the
    /// font could not supply.
    fn codepoints(&self) -> BTreeSet<u32> {
        let manifest = self.atlas.manifest();
        manifest
            .glyphs
            .iter()
            .map(|glyph| glyph.codepoint)
            .chain(
                manifest
                    .generation
                    .failures
                    .iter()
                    .map(|failure| failure.codepoint),
            )
            .collect()
    }
}

/// What this process serves.
#[derive(Default)]
struct CacheState {
    generation: Option<Generation>,
    /// Codepoints the font could not supply that no published manifest
    /// records yet, with the reason. An atlas needs at least one glyph, so a
    /// generation of such codepoints alone is kept here instead; the next
    /// published generation records them.
    unpublished_failures: BTreeMap<u32, String>,
}

impl CacheState {
    fn codepoints(&self) -> BTreeSet<u32> {
        let mut codepoints = self
            .generation
            .as_ref()
            .map(Generation::codepoints)
            .unwrap_or_default();
        codepoints.extend(self.unpublished_failures.keys().copied());
        codepoints
    }

    /// Every failure a new generation carries over without trying again.
    fn known_failures(&self) -> BTreeMap<u32, String> {
        let mut failures = self
            .generation
            .iter()
            .flat_map(|generation| &generation.atlas.manifest().generation.failures)
            .map(|failure| (failure.codepoint, failure.reason.clone()))
            .collect::<BTreeMap<_, _>>();
        failures.extend(
            self.unpublished_failures
                .iter()
                .map(|(codepoint, reason)| (*codepoint, reason.clone())),
        );
        failures
    }
}

/// A generation built for a codepoint set.
enum BuiltGeneration {
    Published(Generation, u64),
    /// Every codepoint failed, which leaves no atlas to publish.
    NoGlyphs(Vec<SdfAtlasGenerationFailure>),
}

pub struct PersistentFallbackSdfCache {
    root: PathBuf,
    font_path: PathBuf,
    font_family: String,
    font_sha256: String,
    generator_contract: String,
    identity: String,
    cmap_codepoint_count: u32,
    state: Mutex<CacheState>,
}

impl PersistentFallbackSdfCache {
    pub fn new(
        root: impl Into<PathBuf>,
        font_path: impl Into<PathBuf>,
        font_family: impl Into<String>,
    ) -> Result<Self, String> {
        let root = root.into();
        let font_path = font_path.into();
        if !font_path.is_file() {
            return Err(format!(
                "fallback font file does not exist: {}",
                font_path.display()
            ));
        }
        fs::create_dir_all(&root).map_err(|error| {
            format!(
                "create fallback cache root {} failed: {error}",
                root.display()
            )
        })?;
        let font_bytes = fs::read(&font_path).map_err(|error| {
            format!("read fallback font {} failed: {error}", font_path.display())
        })?;
        let font_family = font_family.into();
        let font_sha256 = hex::encode(Sha256::digest(&font_bytes));
        let cmap_codepoint_count = count_cmap_codepoints(&font_bytes)?;
        let generator_contract = OfflineGenerationMethod::Edt {
            supersample: FALLBACK_SUPERSAMPLE,
        }
        .contract();
        let identity = cache_identity(&font_family, &font_sha256, &generator_contract);
        let cache = Self {
            root,
            font_path,
            font_family,
            font_sha256,
            generator_contract,
            identity,
            cmap_codepoint_count,
            state: Mutex::new(CacheState::default()),
        };
        let current = {
            let _file_guard = CacheFileLock::acquire(&cache.root)?;
            cache
                .read_current_pointer()?
                .and_then(|pointer| cache.open_generation(pointer.manifest, true))
        };
        cache
            .state
            .lock()
            .map_err(|_| "fallback cache process state is poisoned".to_string())?
            .generation = current;
        Ok(cache)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn font_family(&self) -> &str {
        &self.font_family
    }

    pub fn load_current(&self) -> Result<Option<Arc<MappedSdfAtlas>>, String> {
        self.state
            .lock()
            .map(|state| {
                state
                    .generation
                    .as_ref()
                    .map(|generation| generation.atlas.clone())
            })
            .map_err(|_| "fallback cache process state is poisoned".to_string())
    }

    pub fn ensure_codepoints(
        &self,
        requested: &BTreeSet<u32>,
    ) -> Result<
        (
            Option<Arc<MappedSdfAtlas>>,
            PersistentFallbackSdfCacheReport,
        ),
        String,
    > {
        let started = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "fallback cache process state is poisoned".to_string())?;
        let existing = state.codepoints();
        let mut report = PersistentFallbackSdfCacheReport {
            requested_codepoint_count: requested.len() as u64,
            cache_hit_count: requested.intersection(&existing).count() as u64,
            total_cached_codepoint_count: existing.len() as u64,
            font_family: self.font_family.clone(),
            font_sha256: self.font_sha256.clone(),
            ..PersistentFallbackSdfCacheReport::default()
        };
        if requested.is_subset(&existing) {
            return Ok(finish(state.generation.as_ref(), report, started));
        }

        // A missing in-process glyph may already have been published by
        // another worker. Refresh under the cross-process lock before doing
        // any generation work.
        let _file_guard = CacheFileLock::acquire(&self.root)?;
        let published = match self.read_current_pointer()? {
            None => None,
            Some(pointer)
                if state
                    .generation
                    .as_ref()
                    .is_some_and(|current| current.manifest == pointer.manifest) =>
            {
                state.generation.clone()
            }
            Some(pointer) => self.open_generation(pointer.manifest, false),
        };
        state.generation = published;
        let existing = state.codepoints();
        report.cache_hit_count = requested.intersection(&existing).count() as u64;
        report.total_cached_codepoint_count = existing.len() as u64;
        if requested.is_subset(&existing) {
            return Ok(finish(state.generation.as_ref(), report, started));
        }

        let all_codepoints = existing.union(requested).copied().collect::<BTreeSet<_>>();
        match self.build_generation(&all_codepoints, &state)? {
            BuiltGeneration::Published(generation, generated_file_bytes) => {
                self.publish(&generation)?;
                self.collect_garbage(&generation);
                report.generated_file_bytes = generated_file_bytes;
                // The published manifest records them now.
                state.unpublished_failures.clear();
                state.generation = Some(generation);
            }
            BuiltGeneration::NoGlyphs(failures) => {
                state.unpublished_failures.extend(
                    failures
                        .into_iter()
                        .map(|failure| (failure.codepoint, failure.reason)),
                );
            }
        }
        report.generated_codepoint_count = state.generation.as_ref().map_or(0, |generation| {
            requested
                .difference(&existing)
                .filter(|codepoint| generation.atlas.glyph(**codepoint).is_some())
                .count() as u64
        });
        report.total_cached_codepoint_count = state.codepoints().len() as u64;
        Ok(finish(state.generation.as_ref(), report, started))
    }

    /// The published pointer, or `None` when there is none or it belongs to
    /// another font, generator contract or layout.
    fn read_current_pointer(&self) -> Result<Option<CachePointer>, String> {
        let pointer_path = self.root.join(POINTER_FILE);
        let bytes = match fs::read(&pointer_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "read fallback cache pointer {} failed: {error}",
                    pointer_path.display()
                ))
            }
        };
        let Ok(pointer) = serde_json::from_slice::<CachePointer>(&bytes) else {
            tracing::warn!(path = %pointer_path.display(), "ignoring unreadable fallback cache pointer");
            return Ok(None);
        };
        let matches = pointer.schema == POINTER_SCHEMA
            && pointer.font_family == self.font_family
            && pointer.font_sha256 == self.font_sha256
            && pointer.generator_contract == self.generator_contract
            && self.is_manifest_path(&pointer.manifest);
        Ok(matches.then_some(pointer))
    }

    /// Whether `manifest` names a manifest file directly inside this cache's
    /// identity directory.
    fn is_manifest_path(&self, manifest: &str) -> bool {
        let mut components = Path::new(manifest).components();
        matches!(
            (components.next(), components.next(), components.next()),
            (Some(Component::Normal(directory)), Some(Component::Normal(file)), None)
                if directory == self.identity.as_str()
                    && file.to_str().is_some_and(|file| {
                        file.starts_with(MANIFEST_PREFIX) && file.ends_with(".json")
                    })
        )
    }

    /// Opens a published generation of this cache's identity. A generation
    /// that cannot be opened or belongs to another identity is a cache miss.
    fn open_generation(&self, manifest: String, verify_page_hashes: bool) -> Option<Generation> {
        let manifest_path = self.root.join(&manifest);
        let opened = if verify_page_hashes {
            MappedSdfAtlas::open(&manifest_path)
        } else {
            MappedSdfAtlas::open_trusted_immutable_artifact(&manifest_path)
        };
        match opened {
            Ok(atlas) if self.matches_identity(atlas.manifest()) => Some(Generation {
                manifest,
                atlas: Arc::new(atlas),
            }),
            Ok(_) => {
                tracing::warn!(path = %manifest_path.display(), "ignoring fallback atlas of another identity");
                None
            }
            Err(error) => {
                tracing::warn!(path = %manifest_path.display(), %error, "ignoring unreadable fallback atlas");
                None
            }
        }
    }

    fn matches_identity(&self, manifest: &SdfAtlasManifest) -> bool {
        manifest.font_family == self.font_family
            && manifest.font_sha256 == self.font_sha256
            && manifest.generator_contract == self.generator_contract
            && manifest.point_size == outline::sampling_point_size()
            && manifest.spread == outline::sampling_spread()
    }

    fn build_generation(
        &self,
        codepoints: &BTreeSet<u32>,
        state: &CacheState,
    ) -> Result<BuiltGeneration, String> {
        let mut digest = Sha256::new();
        digest.update(self.identity.as_bytes());
        for codepoint in codepoints {
            digest.update(codepoint.to_le_bytes());
        }
        let manifest = format!(
            "{}/{MANIFEST_PREFIX}{}.json",
            self.identity,
            hex::encode(digest.finalize())
        );
        // Another worker may have built this exact set without publishing it.
        if let Some(generation) = self
            .open_generation(manifest.clone(), false)
            .filter(|generation| generation.codepoints() == *codepoints)
        {
            return Ok(BuiltGeneration::Published(generation, 0));
        }

        let identity_dir = self.root.join(&self.identity);
        let pages_dir = identity_dir.join(PAGES_DIR);
        fs::create_dir_all(&pages_dir).map_err(|error| {
            format!(
                "create fallback page directory {} failed: {error}",
                pages_dir.display()
            )
        })?;
        let (atlas_manifest, mut generated_file_bytes) = self.write_pages(
            &pages_dir,
            codepoints,
            state.generation.as_ref().map(|current| &*current.atlas),
            &state.known_failures(),
        )?;
        if atlas_manifest.glyphs.is_empty() {
            return Ok(BuiltGeneration::NoGlyphs(
                atlas_manifest.generation.failures,
            ));
        }
        let mut manifest_bytes = serde_json::to_vec_pretty(&atlas_manifest)
            .map_err(|error| format!("serialize fallback manifest failed: {error}"))?;
        manifest_bytes.push(b'\n');
        let manifest_path = self.root.join(&manifest);
        write_atomically(&manifest_path, &manifest_bytes)?;
        generated_file_bytes = generated_file_bytes.saturating_add(manifest_bytes.len() as u64);

        // Pages carried over were verified when their generation was opened
        // and new pages were just written, so the reopen only re-validates
        // structure and headers.
        let atlas = MappedSdfAtlas::open_trusted_immutable_artifact(&manifest_path)
            .map_err(|error| format!("reopen fallback atlas failed: {error}"))?;
        if atlas.manifest() != &atlas_manifest {
            return Err("fallback manifest roundtrip mismatch".into());
        }
        Ok(BuiltGeneration::Published(
            Generation {
                manifest,
                atlas: Arc::new(atlas),
            },
            generated_file_bytes,
        ))
    }

    /// Writes a page for every codepoint neither `current` nor
    /// `known_failures` already resolves and returns the manifest of the
    /// complete set. A codepoint the font cannot supply becomes a generation
    /// failure.
    fn write_pages(
        &self,
        pages_dir: &Path,
        codepoints: &BTreeSet<u32>,
        current: Option<&MappedSdfAtlas>,
        known_failures: &BTreeMap<u32, String>,
    ) -> Result<(SdfAtlasManifest, u64), String> {
        let mut generator = None;
        let mut pages = Vec::with_capacity(codepoints.len());
        let mut glyphs = Vec::with_capacity(codepoints.len());
        let mut failures = Vec::new();
        let mut analytic_fallback_codepoints = Vec::new();
        let mut generated_file_bytes = 0u64;
        for codepoint in codepoints.iter().copied() {
            let page_index = u16::try_from(pages.len())
                .map_err(|_| "fallback page count exceeds u16".to_string())?;
            if let Some((atlas, existing)) =
                current.and_then(|atlas| atlas.glyph(codepoint).map(|glyph| (atlas, glyph)))
            {
                let page = atlas
                    .manifest()
                    .pages
                    .get(usize::from(existing.page))
                    .ok_or_else(|| {
                        format!("fallback glyph U+{codepoint:04X} references a missing page")
                    })?;
                pages.push(page.clone());
                glyphs.push(SdfAtlasGlyphManifest {
                    page: page_index,
                    ..existing.clone()
                });
                if atlas
                    .manifest()
                    .generation
                    .analytic_fallback_codepoints
                    .contains(&codepoint)
                {
                    analytic_fallback_codepoints.push(codepoint);
                }
                continue;
            }
            if let Some(reason) = known_failures.get(&codepoint) {
                failures.push(SdfAtlasGenerationFailure {
                    codepoint,
                    reason: reason.clone(),
                });
                continue;
            }
            let generator = match &mut generator {
                Some(generator) => generator,
                empty => empty.insert(OfflineAtlasGlyphGenerator::new_from_path(&self.font_path)?),
            };
            let ch = char::from_u32(codepoint)
                .ok_or_else(|| format!("invalid fallback codepoint U+{codepoint:04X}"))?;
            let (glyph, used_fallback) = match generator.generate(
                ch,
                OfflineGenerationMethod::Edt {
                    supersample: FALLBACK_SUPERSAMPLE,
                },
            ) {
                Ok(generated) => generated,
                Err(reason) => {
                    tracing::debug!(
                        font_family = %self.font_family,
                        codepoint = %format!("U+{codepoint:04X}"),
                        %reason,
                        "fallback font cannot supply the codepoint"
                    );
                    failures.push(SdfAtlasGenerationFailure { codepoint, reason });
                    continue;
                }
            };
            let (page, page_bytes) = write_glyph_page(pages_dir, codepoint, &glyph)?;
            generated_file_bytes = generated_file_bytes.saturating_add(page_bytes);
            glyphs.push(SdfAtlasGlyphManifest {
                codepoint,
                page: page_index,
                rect: [
                    0,
                    0,
                    u32::try_from(glyph.width())
                        .map_err(|_| "fallback glyph width overflow".to_string())?,
                    u32::try_from(glyph.height())
                        .map_err(|_| "fallback glyph height overflow".to_string())?,
                ],
                plane_bearing: [glyph.plane_bearing_x(), glyph.plane_bearing_y()],
                plane_size: [glyph.plane_width(), glyph.plane_height()],
                plane_advance_x: glyph.plane_advance_x(),
            });
            pages.push(page);
            if used_fallback {
                analytic_fallback_codepoints.push(codepoint);
            }
        }
        let manifest = SdfAtlasManifest {
            schema: ATLAS_MANIFEST_SCHEMA.into(),
            generator_contract: self.generator_contract.clone(),
            font_family: self.font_family.clone(),
            font_sha256: self.font_sha256.clone(),
            point_size: outline::sampling_point_size(),
            spread: outline::sampling_spread(),
            generation: SdfAtlasGenerationReport {
                cmap_codepoint_count: self.cmap_codepoint_count,
                requested_codepoint_count: codepoints.len() as u32,
                generated_glyph_count: glyphs.len() as u32,
                failed_glyph_count: failures.len() as u32,
                analytic_fallback_count: analytic_fallback_codepoints.len() as u32,
                page_width: pages.iter().map(|page| page.width).max().unwrap_or(0),
                page_height: pages.iter().map(|page| page.height).max().unwrap_or(0),
                gutter: 0,
                failures,
                analytic_fallback_codepoints,
            },
            pages,
            glyphs,
        };
        Ok((manifest, generated_file_bytes))
    }

    fn publish(&self, generation: &Generation) -> Result<(), String> {
        let pointer = CachePointer {
            schema: POINTER_SCHEMA.into(),
            font_family: self.font_family.clone(),
            font_sha256: self.font_sha256.clone(),
            generator_contract: self.generator_contract.clone(),
            manifest: generation.manifest.clone(),
        };
        let mut bytes = serde_json::to_vec_pretty(&pointer)
            .map_err(|error| format!("serialize fallback cache pointer failed: {error}"))?;
        bytes.push(b'\n');
        write_atomically(&self.root.join(POINTER_FILE), &bytes)
    }

    /// Removes everything the published generation no longer needs. Runs under
    /// the file lock; failures only leave files behind for the next publish.
    fn collect_garbage(&self, published: &Generation) {
        let referenced = published
            .atlas
            .manifest()
            .pages
            .iter()
            .filter_map(|page| Path::new(&page.file).file_name().map(ToOwned::to_owned))
            .collect::<BTreeSet<_>>();
        let identity_dir = self.root.join(&self.identity);
        let published_manifest = Path::new(&published.manifest).file_name();
        for path in directory_entries(&identity_dir) {
            let name = path.file_name();
            let is_manifest_or_temp = name
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(MANIFEST_PREFIX) || name.starts_with('.'));
            if is_manifest_or_temp && name != published_manifest {
                remove_quietly(&path);
            }
        }
        for path in directory_entries(&identity_dir.join(PAGES_DIR)) {
            if path
                .file_name()
                .is_none_or(|name| !referenced.contains(name))
            {
                remove_quietly(&path);
            }
        }
        for path in directory_entries(&self.root) {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let other_identity = name != self.identity && is_sha256_hex(name);
            if other_identity || name == SUPERSEDED_GENERATIONS_DIR {
                remove_quietly(&path);
            }
        }
    }
}

fn finish(
    current: Option<&Generation>,
    mut report: PersistentFallbackSdfCacheReport,
    started: Instant,
) -> (
    Option<Arc<MappedSdfAtlas>>,
    PersistentFallbackSdfCacheReport,
) {
    report.manifest_sha256 = current.map(|current| current.atlas.manifest_sha256().to_string());
    report.elapsed_ns = elapsed_ns(started);
    (current.map(|current| current.atlas.clone()), report)
}

/// Digest of everything a cached glyph depends on besides its codepoint.
fn cache_identity(font_family: &str, font_sha256: &str, generator_contract: &str) -> String {
    let mut digest = Sha256::new();
    for part in [font_family, font_sha256, generator_contract] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    digest.update(outline::sampling_point_size().to_bits().to_le_bytes());
    digest.update(outline::sampling_spread().to_bits().to_le_bytes());
    hex::encode(digest.finalize())
}

fn is_sha256_hex(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn directory_entries(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .collect()
        })
        .unwrap_or_default()
}

fn remove_quietly(path: &Path) {
    let removed = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    if let Err(error) = removed {
        tracing::debug!(path = %path.display(), %error, "fallback cache cleanup skipped a path");
    }
}

/// Writes `bytes` beside `path` and renames the file into place, so readers
/// never observe a partial file.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid fallback cache path {}", path.display()))?;
    let temp = path.with_file_name(format!(".{file_name}.tmp-{}-{nonce}", std::process::id()));
    fs::write(&temp, bytes).map_err(|error| {
        format!(
            "write fallback cache file {} failed: {error}",
            temp.display()
        )
    })?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!(
            "publish fallback cache file {} failed: {error}",
            path.display()
        )
    })
}

/// Exclusive advisory lock on `<root>/cache.lock`.
///
/// The operating system releases the lock when its holder exits, so a crashed
/// worker cannot leave the cache locked, and no holder can remove a lock held
/// by another. The lock file itself is never deleted.
struct CacheFileLock {
    _file: File,
}

impl CacheFileLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        Self::acquire_within(root, LOCK_WAIT_LIMIT)
    }

    fn acquire_within(root: &Path, limit: Duration) -> Result<Self, String> {
        let path = root.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                format!(
                    "open fallback cache lock {} failed: {error}",
                    path.display()
                )
            })?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) if started.elapsed() < limit => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(format!(
                        "timed out waiting for fallback cache lock {}",
                        path.display()
                    ))
                }
                Err(TryLockError::Error(error)) => {
                    return Err(format!(
                        "lock fallback cache {} failed: {error}",
                        path.display()
                    ))
                }
            }
        }
    }
}

fn write_glyph_page(
    pages_dir: &Path,
    codepoint: u32,
    glyph: &OutlineSdfGlyph,
) -> Result<(SdfAtlasPageManifest, u64), String> {
    let width = round_up_to_block(glyph.width(), SWIZZLED_BLOCK_WIDTH as usize)?;
    let height = round_up_to_block(glyph.height(), SWIZZLED_BLOCK_HEIGHT as usize)?;
    let mut linear = vec![0u8; width * height];
    for row in 0..glyph.height() {
        linear[row * width..row * width + glyph.width()]
            .copy_from_slice(&glyph.pixels()[row * glyph.width()..(row + 1) * glyph.width()]);
    }
    let payload = swizzle_page(&linear, width, height);
    let source_hash = Sha256::digest(&linear);
    let mut bytes = vec![0u8; SWIZZLED_PAGE_HEADER_BYTES];
    bytes[..SWIZZLED_PAGE_MAGIC.len()].copy_from_slice(SWIZZLED_PAGE_MAGIC);
    bytes[12..16].copy_from_slice(&SWIZZLED_PAGE_VERSION.to_le_bytes());
    bytes[16..20].copy_from_slice(&(width as u32).to_le_bytes());
    bytes[20..24].copy_from_slice(&(height as u32).to_le_bytes());
    bytes[24..28].copy_from_slice(&SWIZZLED_BLOCK_WIDTH.to_le_bytes());
    bytes[28..32].copy_from_slice(&SWIZZLED_BLOCK_HEIGHT.to_le_bytes());
    bytes[32..64].copy_from_slice(&source_hash);
    bytes.extend_from_slice(&payload);
    let file_name = format!("u{codepoint:08X}.r8swz");
    write_atomically(&pages_dir.join(&file_name), &bytes)?;
    Ok((
        SdfAtlasPageManifest {
            file: format!("{PAGES_DIR}/{file_name}"),
            width: width as u32,
            height: height as u32,
            file_sha256: hex::encode(Sha256::digest(&bytes)),
        },
        bytes.len() as u64,
    ))
}

fn swizzle_page(linear: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut swizzled = vec![0u8; linear.len()];
    let blocks_per_row = width / SWIZZLED_BLOCK_WIDTH as usize;
    for y in 0..height {
        for x in 0..width {
            let block = (y / SWIZZLED_BLOCK_HEIGHT as usize) * blocks_per_row
                + x / SWIZZLED_BLOCK_WIDTH as usize;
            let in_block = (y % SWIZZLED_BLOCK_HEIGHT as usize) * SWIZZLED_BLOCK_WIDTH as usize
                + x % SWIZZLED_BLOCK_WIDTH as usize;
            swizzled[block * 64 + in_block] = linear[y * width + x];
        }
    }
    swizzled
}

fn round_up_to_block(value: usize, block: usize) -> Result<usize, String> {
    value
        .checked_add(block - 1)
        .map(|value| value / block * block)
        .filter(|value| *value > 0)
        .ok_or_else(|| "fallback glyph page dimension overflow".to_string())
}

fn count_cmap_codepoints(font_bytes: &[u8]) -> Result<u32, String> {
    let face = Face::parse(font_bytes, 0)
        .map_err(|error| format!("parse fallback font failed: {error:?}"))?;
    let cmap = face
        .tables()
        .cmap
        .ok_or_else(|| "fallback font has no cmap table".to_string())?;
    let mut codepoints = BTreeSet::new();
    for subtable in cmap.subtables {
        if subtable.is_unicode() {
            subtable.codepoints(|codepoint| {
                if char::from_u32(codepoint).is_some() {
                    codepoints.insert(codepoint);
                }
            });
        }
    }
    u32::try_from(codepoints.len()).map_err(|_| "fallback cmap count exceeds u32".to_string())
}

fn elapsed_ns(started: Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGULAR: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
    const BOLD: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf";

    fn fonts_available() -> bool {
        let available = Path::new(REGULAR).is_file() && Path::new(BOLD).is_file();
        if !available {
            eprintln!("skipping: DejaVu Sans is not installed");
        }
        available
    }

    fn codepoints(text: &str) -> BTreeSet<u32> {
        text.chars().map(u32::from).collect()
    }

    /// Every regular file below `root`, relative to it.
    fn files_below(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).expect("read cache directory") {
                let path = entry.expect("cache directory entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.push(
                        path.strip_prefix(root)
                            .expect("path below root")
                            .to_path_buf(),
                    );
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn extending_the_cache_generates_only_new_glyphs_and_keeps_one_copy_of_each() {
        if !fonts_available() {
            return;
        }
        let root = tempfile::tempdir().expect("fallback cache tempdir");
        let cache = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
            .expect("create fallback cache");
        for (step, text) in ["A", "AB", "ABC"].into_iter().enumerate() {
            let (atlas, report) = cache
                .ensure_codepoints(&codepoints(text))
                .expect("extend fallback cache");
            let atlas = atlas.expect("fallback atlas");
            assert_eq!(atlas.manifest().glyphs.len(), step + 1);
            assert_eq!(report.generated_codepoint_count, 1);
            assert_eq!(report.cache_hit_count, step as u64);
        }
        let (_, report) = cache
            .ensure_codepoints(&codepoints("CA"))
            .expect("hit fallback cache");
        assert_eq!(report.generated_codepoint_count, 0);
        assert_eq!(report.cache_hit_count, 2);

        // One page per cached glyph and one manifest; superseded
        // generations are not left behind.
        let files = files_below(root.path());
        let pages = files
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "r8swz"))
            .count();
        let manifests = files
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .filter(|path| !path.ends_with("current.json"))
            .count();
        assert_eq!((pages, manifests), (3, 1), "cache files: {files:?}");

        let reopened = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
            .expect("reopen fallback cache");
        let current = reopened
            .load_current()
            .expect("cache state")
            .expect("persisted atlas");
        assert_eq!(current.manifest().glyphs.len(), 3);
    }

    #[test]
    fn codepoints_the_font_lacks_are_recorded_instead_of_failing_the_generation() {
        if !fonts_available() {
            return;
        }
        let root = tempfile::tempdir().expect("fallback cache tempdir");
        let cache = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
            .expect("create fallback cache");
        // DejaVu Sans has no CJK ideographs.
        let requested = codepoints("A\u{6F22}");
        let (atlas, report) = cache
            .ensure_codepoints(&requested)
            .expect("a codepoint the font lacks must not fail the generation");
        let atlas = atlas.expect("fallback atlas");
        assert!(atlas.glyph(u32::from('A')).is_some());
        assert!(atlas.glyph(0x6F22).is_none());
        let failures = atlas
            .manifest()
            .generation
            .failures
            .iter()
            .map(|failure| failure.codepoint)
            .collect::<Vec<_>>();
        assert_eq!(failures, vec![0x6F22]);
        assert_eq!(report.generated_codepoint_count, 1);

        // The recorded codepoint counts as resolved: asking again, in this
        // process or after a restart, generates nothing.
        let (again, report) = cache
            .ensure_codepoints(&requested)
            .expect("repeated request");
        assert_eq!(
            (report.cache_hit_count, report.generated_codepoint_count),
            (2, 0)
        );
        assert_eq!(
            again.expect("fallback atlas").manifest_sha256(),
            atlas.manifest_sha256()
        );
        let reopened = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
            .expect("reopen fallback cache");
        let (_, report) = reopened
            .ensure_codepoints(&requested)
            .expect("request after reopening");
        assert_eq!(
            (report.cache_hit_count, report.generated_codepoint_count),
            (2, 0)
        );

        // With no glyph at all there is no atlas to publish, and the codepoint
        // is still not generated again.
        let root = tempfile::tempdir().expect("second fallback cache tempdir");
        let cache = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
            .expect("create second fallback cache");
        let unavailable = codepoints("\u{6F22}");
        let (atlas, _) = cache
            .ensure_codepoints(&unavailable)
            .expect("a request of unavailable codepoints only");
        assert!(atlas.is_none());
        let (_, report) = cache
            .ensure_codepoints(&unavailable)
            .expect("repeated unavailable request");
        assert_eq!(
            (report.cache_hit_count, report.generated_codepoint_count),
            (1, 0)
        );
    }

    #[test]
    fn replacing_the_font_file_invalidates_its_cached_glyphs() {
        if !fonts_available() {
            return;
        }
        let root = tempfile::tempdir().expect("fallback cache tempdir");
        let font_dir = tempfile::tempdir().expect("font tempdir");
        let font = font_dir.path().join("fallback.ttf");
        fs::copy(REGULAR, &font).expect("install regular font");
        let regular = PersistentFallbackSdfCache::new(root.path(), &font, "test-fallback")
            .expect("create fallback cache");
        let (regular_atlas, _) = regular
            .ensure_codepoints(&codepoints("A"))
            .expect("regular glyph");
        let regular_atlas = regular_atlas.expect("regular atlas");

        fs::copy(BOLD, &font).expect("replace font file");
        let bold_sha256 = hex::encode(Sha256::digest(
            fs::read(&font).expect("replaced font bytes"),
        ));
        let bold = PersistentFallbackSdfCache::new(root.path(), &font, "test-fallback")
            .expect("reopen fallback cache for the replaced font");
        assert!(
            bold.load_current().expect("cache state").is_none(),
            "glyphs of the previous font must not be served"
        );
        let (bold_atlas, report) = bold
            .ensure_codepoints(&codepoints("A"))
            .expect("bold glyph");
        let bold_atlas = bold_atlas.expect("bold atlas");
        assert_eq!(report.generated_codepoint_count, 1);
        assert_eq!(bold_atlas.manifest().font_sha256, bold_sha256);
        assert_ne!(
            bold_atlas.manifest().glyphs[0].plane_advance_x,
            regular_atlas.manifest().glyphs[0].plane_advance_x
        );
        let identity_dirs = fs::read_dir(root.path())
            .expect("cache root listing")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .count();
        assert_eq!(identity_dirs, 1, "the previous font's glyphs are reclaimed");
    }

    #[test]
    fn a_pointer_from_another_generator_contract_is_a_cache_miss() {
        if !fonts_available() {
            return;
        }
        let root = tempfile::tempdir().expect("fallback cache tempdir");
        fs::write(
            root.path().join("current.json"),
            serde_json::json!({
                "schema": "allium.sdf-fallback-cache-pointer.v1",
                "font_family": "test-fallback",
                "font_sha256": "00".repeat(32),
                "generator_contract": "outline-edt-v0",
                "manifest": "generations/old/manifest.json",
            })
            .to_string(),
        )
        .expect("foreign pointer");
        let cache = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
            .expect("a foreign pointer must not prevent opening the cache");
        assert!(cache.load_current().expect("cache state").is_none());
        let (atlas, _) = cache.ensure_codepoints(&codepoints("A")).expect("glyph");
        assert!(atlas.is_some());
    }

    #[test]
    fn a_lock_file_left_by_a_crashed_process_does_not_block() {
        let root = tempfile::tempdir().expect("lock tempdir");
        fs::write(root.path().join("cache.lock"), "pid=1\n").expect("stale lock file");
        let started = Instant::now();
        let guard = CacheFileLock::acquire(root.path()).expect("acquire lock");
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(guard);
        assert!(CacheFileLock::acquire(root.path()).is_ok());
    }

    #[test]
    fn the_lock_admits_one_holder_at_a_time() {
        let root = tempfile::tempdir().expect("lock tempdir");
        let first = CacheFileLock::acquire(root.path()).expect("first holder");
        assert!(CacheFileLock::acquire_within(root.path(), Duration::from_millis(50)).is_err());
        drop(first);
        let second = CacheFileLock::acquire_within(root.path(), Duration::from_millis(50))
            .expect("lock is free once the first holder is gone");
        assert!(CacheFileLock::acquire_within(root.path(), Duration::from_millis(50)).is_err());
        drop(second);
    }
}
