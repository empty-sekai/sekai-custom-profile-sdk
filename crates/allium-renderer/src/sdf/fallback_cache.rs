//! Sparse, persistent fallback glyph cache.
//!
//! Only codepoints that miss their selected profile font are generated. Each
//! glyph is stored in its own minimal 8x8-aligned mmap page so adding one
//! fallback never requires generating the fallback font's complete cmap.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ttf_parser::Face;

use super::atlas::{
    MappedSdfAtlas, SdfAtlasGenerationReport, SdfAtlasGlyphManifest, SdfAtlasManifest,
    SdfAtlasPageManifest, ATLAS_MANIFEST_SCHEMA, PROFILE_TEXT_FALLBACK_FONT_FAMILY,
    SWIZZLED_BLOCK_HEIGHT, SWIZZLED_BLOCK_WIDTH, SWIZZLED_PAGE_HEADER_BYTES, SWIZZLED_PAGE_MAGIC,
    SWIZZLED_PAGE_VERSION,
};
use super::outline::{self, OfflineAtlasGlyphGenerator, OfflineGenerationMethod, OutlineSdfGlyph};

pub const SOURCE_HAN_SANS_SC_FAMILY: &str = PROFILE_TEXT_FALLBACK_FONT_FAMILY;
pub const FALLBACK_GENERATOR_CONTRACT: &str = "outline-edt-v1:ss=2:fallback=analytic-v1";
const POINTER_SCHEMA: &str = "allium.sdf-fallback-cache-pointer.v1";
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(30);
const STALE_LOCK_AGE: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistentFallbackSdfCacheReport {
    pub requested_codepoint_count: u64,
    pub cache_hit_count: u64,
    pub generated_codepoint_count: u64,
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

pub struct PersistentFallbackSdfCache {
    root: PathBuf,
    font_path: PathBuf,
    font_family: String,
    font_sha256: String,
    cmap_codepoint_count: u32,
    current: Mutex<Option<std::sync::Arc<MappedSdfAtlas>>>,
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
        fs::create_dir_all(root.join("generations")).map_err(|error| {
            format!(
                "create fallback cache root {} failed: {error}",
                root.display()
            )
        })?;
        let font_bytes = fs::read(&font_path).map_err(|error| {
            format!("read fallback font {} failed: {error}", font_path.display())
        })?;
        let font_sha256 = hex::encode(Sha256::digest(&font_bytes));
        let cmap_codepoint_count = count_cmap_codepoints(&font_bytes)?;
        let cache = Self {
            root,
            font_path,
            font_family: font_family.into(),
            font_sha256,
            cmap_codepoint_count,
            current: Mutex::new(None),
        };
        let current = cache.load_current_from_disk()?;
        *cache
            .current
            .lock()
            .map_err(|_| "fallback cache process state is poisoned".to_string())? = current;
        Ok(cache)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn font_family(&self) -> &str {
        &self.font_family
    }

    pub fn load_current(&self) -> Result<Option<std::sync::Arc<MappedSdfAtlas>>, String> {
        self.current
            .lock()
            .map(|current| current.clone())
            .map_err(|_| "fallback cache process state is poisoned".to_string())
    }

    fn load_current_from_disk(&self) -> Result<Option<std::sync::Arc<MappedSdfAtlas>>, String> {
        let Some(pointer) = self.read_current_pointer()? else {
            return Ok(None);
        };
        self.open_pointer_atlas(&pointer).map(Some)
    }

    fn read_current_pointer(&self) -> Result<Option<CachePointer>, String> {
        let pointer_path = self.root.join("current.json");
        if !pointer_path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&pointer_path).map_err(|error| {
            format!(
                "read fallback cache pointer {} failed: {error}",
                pointer_path.display()
            )
        })?;
        let pointer: CachePointer = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "parse fallback cache pointer {} failed: {error}",
                pointer_path.display()
            )
        })?;
        self.validate_pointer(&pointer)?;
        Ok(Some(pointer))
    }

    fn open_pointer_atlas(
        &self,
        pointer: &CachePointer,
    ) -> Result<std::sync::Arc<MappedSdfAtlas>, String> {
        let manifest_path = self.root.join(&pointer.manifest);
        let atlas = MappedSdfAtlas::open(&manifest_path).map_err(|error| {
            format!(
                "open fallback cache atlas {} failed: {error}",
                manifest_path.display()
            )
        })?;
        self.validate_atlas_identity(&atlas, &pointer.font_sha256)?;
        Ok(std::sync::Arc::new(atlas))
    }

    pub fn ensure_codepoints(
        &self,
        requested: &BTreeSet<u32>,
    ) -> Result<
        (
            Option<std::sync::Arc<MappedSdfAtlas>>,
            PersistentFallbackSdfCacheReport,
        ),
        String,
    > {
        let started = Instant::now();
        let mut process_current = self
            .current
            .lock()
            .map_err(|_| "fallback cache process state is poisoned".to_string())?;
        let mut current = process_current.clone();
        let existing = current
            .as_deref()
            .map(|atlas| {
                atlas
                    .manifest()
                    .glyphs
                    .iter()
                    .map(|glyph| glyph.codepoint)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let missing = requested
            .difference(&existing)
            .copied()
            .collect::<BTreeSet<_>>();
        let mut report = PersistentFallbackSdfCacheReport {
            requested_codepoint_count: requested.len() as u64,
            cache_hit_count: requested.intersection(&existing).count() as u64,
            total_cached_codepoint_count: existing.len() as u64,
            font_family: self.font_family.clone(),
            font_sha256: self.font_sha256.clone(),
            ..PersistentFallbackSdfCacheReport::default()
        };
        if missing.is_empty() {
            report.manifest_sha256 = current
                .as_deref()
                .map(MappedSdfAtlas::manifest_sha256)
                .map(str::to_string);
            report.elapsed_ns = elapsed_ns(started);
            return Ok((current, report));
        }

        // A missing in-process glyph may already have been published by
        // another worker. Refresh under the cross-process lock before doing
        // any generation work.
        let _file_guard = CacheFileLock::acquire(&self.root)?;
        let current_pointer = self.read_current_pointer()?;
        current = current_pointer
            .as_ref()
            .map(|pointer| self.open_pointer_atlas(pointer))
            .transpose()?;
        *process_current = current.clone();
        let existing = current
            .as_deref()
            .map(|atlas| {
                atlas
                    .manifest()
                    .glyphs
                    .iter()
                    .map(|glyph| glyph.codepoint)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let missing = requested
            .difference(&existing)
            .copied()
            .collect::<BTreeSet<_>>();
        report.cache_hit_count = requested.intersection(&existing).count() as u64;
        report.total_cached_codepoint_count = existing.len() as u64;
        if missing.is_empty() {
            report.manifest_sha256 = current
                .as_deref()
                .map(MappedSdfAtlas::manifest_sha256)
                .map(str::to_string);
            report.elapsed_ns = elapsed_ns(started);
            return Ok((current, report));
        }

        let all_codepoints = existing.union(requested).copied().collect::<BTreeSet<_>>();
        let current_root = current_pointer.as_ref().and_then(|pointer| {
            self.root
                .join(&pointer.manifest)
                .parent()
                .map(Path::to_path_buf)
        });
        let built = self.build_generation(
            &self.font_sha256,
            &all_codepoints,
            current.as_deref().zip(current_root.as_deref()),
        )?;
        report.generated_codepoint_count = missing.len() as u64;
        report.total_cached_codepoint_count = all_codepoints.len() as u64;
        report.generated_file_bytes = built.generated_file_bytes;
        report.manifest_sha256 = Some(built.atlas.manifest_sha256().to_string());
        report.elapsed_ns = elapsed_ns(started);
        *process_current = Some(built.atlas.clone());
        Ok((Some(built.atlas), report))
    }

    fn validate_pointer(&self, pointer: &CachePointer) -> Result<(), String> {
        if pointer.schema != POINTER_SCHEMA
            || pointer.font_family != self.font_family
            || pointer.generator_contract != FALLBACK_GENERATOR_CONTRACT
        {
            return Err("fallback cache pointer contract mismatch".into());
        }
        let path = Path::new(&pointer.manifest);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err("fallback cache pointer contains an unsafe manifest path".into());
        }
        Ok(())
    }

    fn validate_atlas_identity(
        &self,
        atlas: &MappedSdfAtlas,
        expected_font_sha256: &str,
    ) -> Result<(), String> {
        let manifest = atlas.manifest();
        if manifest.font_family != self.font_family
            || manifest.font_sha256 != expected_font_sha256
            || manifest.generator_contract != FALLBACK_GENERATOR_CONTRACT
        {
            return Err("fallback atlas identity does not match its configured font".into());
        }
        Ok(())
    }

    fn build_generation(
        &self,
        font_sha256: &str,
        codepoints: &BTreeSet<u32>,
        current: Option<(&MappedSdfAtlas, &Path)>,
    ) -> Result<BuiltGeneration, String> {
        let mut identity = Sha256::new();
        identity.update(self.font_family.as_bytes());
        identity.update(font_sha256.as_bytes());
        identity.update(FALLBACK_GENERATOR_CONTRACT.as_bytes());
        for codepoint in codepoints {
            identity.update(codepoint.to_le_bytes());
        }
        let generation_id = hex::encode(identity.finalize());
        let relative_manifest = format!("generations/{generation_id}/manifest.json");
        let generation_root = self.root.join("generations").join(&generation_id);
        let manifest_path = generation_root.join("manifest.json");
        let mut generated_file_bytes = 0u64;

        if !manifest_path.exists() {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let staging = self.root.join("generations").join(format!(
                ".{generation_id}.tmp-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&staging).map_err(|error| {
                format!(
                    "create fallback generation staging {} failed: {error}",
                    staging.display()
                )
            })?;
            let build_result = self.write_generation(&staging, font_sha256, codepoints, current);
            let (manifest, bytes) = match build_result {
                Ok(result) => result,
                Err(error) => {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(error);
                }
            };
            generated_file_bytes = bytes;
            let staged_manifest = staging.join("manifest.json");
            let reopened = MappedSdfAtlas::open(&staged_manifest)
                .map_err(|error| format!("reopen staged fallback atlas failed: {error}"))?;
            if reopened.manifest() != &manifest {
                let _ = fs::remove_dir_all(&staging);
                return Err("staged fallback manifest roundtrip mismatch".into());
            }
            fs::rename(&staging, &generation_root).map_err(|error| {
                format!(
                    "publish fallback generation {} failed: {error}",
                    generation_root.display()
                )
            })?;
        }

        let pointer = CachePointer {
            schema: POINTER_SCHEMA.into(),
            font_family: self.font_family.clone(),
            font_sha256: font_sha256.into(),
            generator_contract: FALLBACK_GENERATOR_CONTRACT.into(),
            manifest: relative_manifest,
        };
        let pointer_path = self.root.join("current.json");
        let pointer_temp = self
            .root
            .join(format!(".current-{}.tmp", std::process::id()));
        let mut pointer_bytes = serde_json::to_vec_pretty(&pointer)
            .map_err(|error| format!("serialize fallback cache pointer failed: {error}"))?;
        pointer_bytes.push(b'\n');
        fs::write(&pointer_temp, pointer_bytes).map_err(|error| {
            format!(
                "write fallback cache pointer {} failed: {error}",
                pointer_temp.display()
            )
        })?;
        fs::rename(&pointer_temp, &pointer_path).map_err(|error| {
            format!(
                "publish fallback cache pointer {} failed: {error}",
                pointer_path.display()
            )
        })?;
        let atlas = std::sync::Arc::new(
            MappedSdfAtlas::open(&manifest_path)
                .map_err(|error| format!("open published fallback atlas failed: {error}"))?,
        );
        self.validate_atlas_identity(&atlas, font_sha256)?;
        Ok(BuiltGeneration {
            atlas,
            generated_file_bytes,
        })
    }

    fn write_generation(
        &self,
        output: &Path,
        font_sha256: &str,
        codepoints: &BTreeSet<u32>,
        current: Option<(&MappedSdfAtlas, &Path)>,
    ) -> Result<(SdfAtlasManifest, u64), String> {
        let generator = OfflineAtlasGlyphGenerator::new_from_path(&self.font_path)?;
        let mut pages = Vec::with_capacity(codepoints.len());
        let mut glyphs = Vec::with_capacity(codepoints.len());
        let mut analytic_fallback_codepoints = Vec::new();
        let mut generated_file_bytes = 0u64;
        let mut max_page_width = 0u32;
        let mut max_page_height = 0u32;
        for codepoint in codepoints.iter().copied() {
            if let Some((atlas, root)) = current {
                if let Some(existing_glyph) = atlas.glyph(codepoint) {
                    let existing_page = atlas
                        .manifest()
                        .pages
                        .get(usize::from(existing_glyph.page))
                        .ok_or_else(|| {
                            format!("fallback glyph U+{codepoint:04X} references a missing page")
                        })?;
                    let destination = output.join(&existing_page.file);
                    fs::hard_link(root.join(&existing_page.file), &destination)
                        .or_else(|_| {
                            fs::copy(root.join(&existing_page.file), &destination).map(|_| ())
                        })
                        .map_err(|error| {
                            format!(
                                "reuse persisted fallback page for U+{codepoint:04X} failed: {error}"
                            )
                        })?;
                    let page_index = u16::try_from(pages.len())
                        .map_err(|_| "fallback page count exceeds u16".to_string())?;
                    let mut reused_glyph = existing_glyph.clone();
                    reused_glyph.page = page_index;
                    max_page_width = max_page_width.max(existing_page.width);
                    max_page_height = max_page_height.max(existing_page.height);
                    pages.push(existing_page.clone());
                    glyphs.push(reused_glyph);
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
            }
            let ch = char::from_u32(codepoint)
                .ok_or_else(|| format!("invalid fallback codepoint U+{codepoint:04X}"))?;
            let (glyph, used_fallback) = generator
                .generate(ch, OfflineGenerationMethod::Edt { supersample: 2 })
                .map_err(|error| {
                    format!("generate fallback glyph U+{codepoint:04X} failed: {error}")
                })?;
            let page_index = u16::try_from(pages.len())
                .map_err(|_| "fallback page count exceeds u16".to_string())?;
            let (page, page_bytes) = write_glyph_page(output, codepoint, &glyph)?;
            max_page_width = max_page_width.max(page.width);
            max_page_height = max_page_height.max(page.height);
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
            generator_contract: FALLBACK_GENERATOR_CONTRACT.into(),
            font_family: self.font_family.clone(),
            font_sha256: font_sha256.into(),
            point_size: outline::sampling_point_size(),
            spread: outline::sampling_spread(),
            pages,
            glyphs,
            generation: SdfAtlasGenerationReport {
                cmap_codepoint_count: self.cmap_codepoint_count,
                requested_codepoint_count: codepoints.len() as u32,
                generated_glyph_count: codepoints.len() as u32,
                failed_glyph_count: 0,
                analytic_fallback_count: analytic_fallback_codepoints.len() as u32,
                page_width: max_page_width,
                page_height: max_page_height,
                gutter: 0,
                failures: Vec::new(),
                analytic_fallback_codepoints,
            },
        };
        let manifest_path = output.join("manifest.json");
        let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("serialize fallback manifest failed: {error}"))?;
        manifest_bytes.push(b'\n');
        fs::write(&manifest_path, &manifest_bytes).map_err(|error| {
            format!(
                "write fallback manifest {} failed: {error}",
                manifest_path.display()
            )
        })?;
        generated_file_bytes = generated_file_bytes.saturating_add(manifest_bytes.len() as u64);
        Ok((manifest, generated_file_bytes))
    }
}

struct BuiltGeneration {
    atlas: std::sync::Arc<MappedSdfAtlas>,
    generated_file_bytes: u64,
}

struct CacheFileLock {
    path: PathBuf,
}

impl CacheFileLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        let path = root.join("cache.lock");
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "pid={}", std::process::id())
                        .map_err(|error| format!("write fallback cache lock failed: {error}"))?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > STALE_LOCK_AGE);
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() >= LOCK_WAIT_LIMIT {
                        return Err(format!(
                            "timed out waiting for fallback cache lock {}",
                            path.display()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(format!(
                        "create fallback cache lock {} failed: {error}",
                        path.display()
                    ))
                }
            }
        }
    }
}

impl Drop for CacheFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_glyph_page(
    output: &Path,
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
    let file = format!("u{codepoint:08X}.r8swz");
    let path = output.join(&file);
    fs::write(&path, &bytes)
        .map_err(|error| format!("write fallback page {} failed: {error}", path.display()))?;
    Ok((
        SdfAtlasPageManifest {
            file,
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

    #[test]
    #[ignore = "static font is not shipped in the OSS repository"]
    fn generates_only_requested_glyph_and_reuses_persisted_generation() {
        let root = tempfile::tempdir().expect("fallback cache tempdir");
        let font = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts/FOT-RodinNTLGPro-DB.ttf");
        let cache = PersistentFallbackSdfCache::new(root.path(), font, "test-fallback")
            .expect("create fallback cache");
        let requested = BTreeSet::from([u32::from('A')]);
        let (first, first_report) = cache
            .ensure_codepoints(&requested)
            .expect("first fallback generation");
        let first = first.expect("generated atlas");
        assert_eq!(first.manifest().glyphs.len(), 1);
        assert!(first.glyph(u32::from('A')).is_some());
        assert_eq!(first_report.generated_codepoint_count, 1);
        assert_eq!(first_report.cache_hit_count, 0);

        let (second, second_report) = cache
            .ensure_codepoints(&requested)
            .expect("reuse fallback generation");
        let second = second.expect("persisted atlas");
        assert_eq!(second.manifest_sha256(), first.manifest_sha256());
        assert_eq!(second_report.generated_codepoint_count, 0);
        assert_eq!(second_report.cache_hit_count, 1);
        assert_eq!(second_report.total_cached_codepoint_count, 1);

        let expanded = BTreeSet::from([u32::from('A'), u32::from('B')]);
        let (third, third_report) = cache
            .ensure_codepoints(&expanded)
            .expect("extend fallback generation");
        let third = third.expect("expanded atlas");
        assert_eq!(third.manifest().glyphs.len(), 2);
        assert!(third.glyph(u32::from('A')).is_some());
        assert!(third.glyph(u32::from('B')).is_some());
        assert_eq!(third_report.generated_codepoint_count, 1);
        assert_eq!(third_report.cache_hit_count, 1);
        assert_eq!(third_report.total_cached_codepoint_count, 2);
    }
}
