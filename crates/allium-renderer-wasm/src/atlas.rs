use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AtlasConfig {
    pub page_width: usize,
    pub page_height: usize,
    pub soft_pages: usize,
    pub hard_pages: usize,
}

impl Default for AtlasConfig {
    fn default() -> Self {
        Self {
            page_width: 2048,
            page_height: 2048,
            soft_pages: 4,
            hard_pages: 6,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AtlasBitmap {
    pub key: String,
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PixelRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasPlacement {
    pub page: usize,
    pub page_epoch: u64,
    pub pixel_rect: PixelRect,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasPageUpdateMeta {
    pub page: usize,
    pub page_width: usize,
    pub page_epoch: u64,
    pub revision: u64,
    pub full_upload: bool,
    pub dirty_rects: Vec<PixelRect>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasStats {
    pub pages: usize,
    pub pinned_pages: usize,
    pub glyphs: usize,
    pub atlas_bytes: usize,
    pub evictions: u64,
    pub page_allocations: u64,
    pub hard_budget_bytes: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasRasterRecord {
    pub key: String,
    pub width: usize,
    pub height: usize,
    pub pixels_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasResolveRequest {
    #[serde(default)]
    pub keys: Vec<String>,
    pub records: Vec<AtlasRasterRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasResolvedGlyph {
    pub key: String,
    pub placement: AtlasPlacement,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasResolveResponse {
    pub lease: Option<u32>,
    pub placements: Vec<AtlasResolvedGlyph>,
    pub missing_keys: Vec<String>,
    pub stats: AtlasStats,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasRevision {
    pub page: usize,
    pub revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasPagesRequest {
    pub revisions: Vec<AtlasRevision>,
}

#[derive(Debug, PartialEq)]
pub enum AtlasError {
    InvalidConfig,
    InvalidBitmap(String),
    MissingGlyph(String),
    MemoryBudgetExceeded {
        pages: usize,
        pinned_pages: usize,
        hard_budget_bytes: usize,
    },
}

impl std::fmt::Display for AtlasError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig => write!(formatter, "invalid atlas dimensions or page budget"),
            Self::InvalidBitmap(key) => write!(formatter, "glyph {key} does not fit the atlas page or has an invalid R8 payload"),
            Self::MissingGlyph(key) => write!(formatter, "glyph {key} is not placed in the atlas"),
            Self::MemoryBudgetExceeded { pages, pinned_pages, hard_budget_bytes } => write!(
                formatter,
                "MEMORY_BUDGET_EXCEEDED: all {pages} atlas pages are pinned ({pinned_pages}) at the {hard_budget_bytes}-byte hard budget"
            ),
        }
    }
}

struct PageChange {
    revision: u64,
    rect: PixelRect,
}

struct AtlasPage {
    epoch: u64,
    revision: u64,
    generation_start_revision: u64,
    pixels: Vec<u8>,
    x: usize,
    y: usize,
    row_height: usize,
    pins: usize,
    last_used: u64,
    glyph_keys: BTreeSet<String>,
    changes: Vec<PageChange>,
}

pub struct AtlasSession {
    config: AtlasConfig,
    pages: Vec<AtlasPage>,
    placements: BTreeMap<String, AtlasPlacement>,
    leases: BTreeMap<u32, Vec<usize>>,
    next_lease: u32,
    clock: u64,
    revision: u64,
    evictions: u64,
    page_allocations: u64,
}

impl AtlasSession {
    pub fn new(config: AtlasConfig) -> Result<Self, AtlasError> {
        if config.page_width < 3
            || config.page_height < 3
            || config.soft_pages < 1
            || config.hard_pages < config.soft_pages
            || config
                .page_width
                .checked_mul(config.page_height)
                .and_then(|bytes| bytes.checked_mul(config.hard_pages))
                .is_none()
        {
            return Err(AtlasError::InvalidConfig);
        }
        Ok(Self {
            config,
            pages: Vec::new(),
            placements: BTreeMap::new(),
            leases: BTreeMap::new(),
            next_lease: 1,
            clock: 0,
            revision: 0,
            evictions: 0,
            page_allocations: 0,
        })
    }

    pub fn put(&mut self, glyph: AtlasBitmap) -> Result<AtlasPlacement, AtlasError> {
        if let Some(existing) = self.placements.get(&glyph.key).cloned() {
            self.touch(existing.page);
            return Ok(existing);
        }
        if glyph.width < 1
            || glyph.height < 1
            || glyph.width + 2 > self.config.page_width
            || glyph.height + 2 > self.config.page_height
            || glyph.pixels.len() != glyph.width * glyph.height
        {
            return Err(AtlasError::InvalidBitmap(glyph.key));
        }
        let mut page_index = self
            .pages
            .iter()
            .position(|page| can_fit(page, &glyph, self.config));
        if page_index.is_none() {
            page_index = Some(self.allocate_or_recycle_page()?);
        }
        let page_index = page_index.expect("atlas page index");
        let (x, y, epoch, revision) = {
            let page = &mut self.pages[page_index];
            prepare_row(page, glyph.width, self.config.page_width);
            let x = page.x;
            let y = page.y;
            for row in 0..glyph.height {
                let source = row * glyph.width;
                let destination = (y + row) * self.config.page_width + x;
                page.pixels[destination..destination + glyph.width]
                    .copy_from_slice(&glyph.pixels[source..source + glyph.width]);
            }
            page.x += glyph.width + 1;
            page.row_height = page.row_height.max(glyph.height);
            page.glyph_keys.insert(glyph.key.clone());
            self.revision += 1;
            page.revision = self.revision;
            page.changes.push(PageChange {
                revision: page.revision,
                rect: PixelRect {
                    x,
                    y,
                    width: glyph.width,
                    height: glyph.height,
                },
            });
            (x, y, page.epoch, page.revision)
        };
        self.touch(page_index);
        let placement = AtlasPlacement {
            page: page_index,
            page_epoch: epoch,
            pixel_rect: PixelRect {
                x,
                y,
                width: glyph.width,
                height: glyph.height,
            },
            u0: x as f32 / self.config.page_width as f32,
            v0: y as f32 / self.config.page_height as f32,
            u1: (x + glyph.width) as f32 / self.config.page_width as f32,
            v1: (y + glyph.height) as f32 / self.config.page_height as f32,
        };
        debug_assert_eq!(self.pages[page_index].revision, revision);
        self.placements.insert(glyph.key, placement.clone());
        Ok(placement)
    }

    pub fn placement(&self, key: &str) -> Option<AtlasPlacement> {
        self.placements.get(key).cloned()
    }

    pub fn acquire(&mut self, keys: &[String]) -> Result<u32, AtlasError> {
        let mut pages = keys
            .iter()
            .map(|key| {
                self.placements
                    .get(key)
                    .map(|placement| placement.page)
                    .ok_or_else(|| AtlasError::MissingGlyph(key.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        pages.sort_unstable();
        pages.dedup();
        for page_index in &pages {
            self.pages[*page_index].pins += 1;
            self.touch(*page_index);
        }
        let lease = self.next_lease;
        self.next_lease = self.next_lease.checked_add(1).unwrap_or(1);
        self.leases.insert(lease, pages);
        Ok(lease)
    }

    pub fn release(&mut self, lease: u32) -> bool {
        let Some(pages) = self.leases.remove(&lease) else {
            return false;
        };
        for page_index in pages {
            self.pages[page_index].pins = self.pages[page_index].pins.saturating_sub(1);
            self.touch(page_index);
        }
        true
    }

    pub fn pages_since(&self, revisions: &BTreeMap<usize, u64>) -> Vec<AtlasPageUpdateMeta> {
        self.pages
            .iter()
            .enumerate()
            .filter_map(|(page_index, page)| {
                let known = revisions.get(&page_index).copied().unwrap_or(0);
                if known >= page.revision {
                    return None;
                }
                let full_upload = known < page.generation_start_revision;
                Some(AtlasPageUpdateMeta {
                    page: page_index,
                    page_width: self.config.page_width,
                    page_epoch: page.epoch,
                    revision: page.revision,
                    full_upload,
                    dirty_rects: if full_upload {
                        Vec::new()
                    } else {
                        page.changes
                            .iter()
                            .filter(|change| change.revision > known)
                            .map(|change| change.rect)
                            .collect()
                    },
                })
            })
            .collect()
    }

    pub fn page_pixels(&self, page: usize) -> Option<&[u8]> {
        self.pages.get(page).map(|page| page.pixels.as_slice())
    }

    pub fn stats(&self) -> AtlasStats {
        let page_bytes = self.config.page_width * self.config.page_height;
        AtlasStats {
            pages: self.pages.len(),
            pinned_pages: self.pages.iter().filter(|page| page.pins > 0).count(),
            glyphs: self.placements.len(),
            atlas_bytes: self.pages.len() * page_bytes,
            evictions: self.evictions,
            page_allocations: self.page_allocations,
            hard_budget_bytes: self.config.hard_pages * page_bytes,
        }
    }

    fn allocate_or_recycle_page(&mut self) -> Result<usize, AtlasError> {
        if self.pages.len() < self.config.hard_pages {
            self.revision += 1;
            let page_index = self.pages.len();
            self.pages.push(AtlasPage {
                epoch: 1,
                revision: self.revision,
                generation_start_revision: self.revision,
                pixels: vec![0; self.config.page_width * self.config.page_height],
                x: 1,
                y: 1,
                row_height: 0,
                pins: 0,
                last_used: 0,
                glyph_keys: BTreeSet::new(),
                changes: Vec::new(),
            });
            self.page_allocations += 1;
            self.touch(page_index);
            return Ok(page_index);
        }
        let Some(page_index) = self
            .pages
            .iter()
            .enumerate()
            .filter(|(_, page)| page.pins == 0)
            .min_by_key(|(index, page)| (page.last_used, *index))
            .map(|(index, _)| index)
        else {
            let stats = self.stats();
            return Err(AtlasError::MemoryBudgetExceeded {
                pages: stats.pages,
                pinned_pages: stats.pinned_pages,
                hard_budget_bytes: stats.hard_budget_bytes,
            });
        };
        let keys = std::mem::take(&mut self.pages[page_index].glyph_keys);
        for key in keys {
            self.placements.remove(&key);
        }
        self.revision += 1;
        let page = &mut self.pages[page_index];
        page.epoch += 1;
        page.revision = self.revision;
        page.generation_start_revision = self.revision;
        page.pixels.fill(0);
        page.x = 1;
        page.y = 1;
        page.row_height = 0;
        page.changes.clear();
        self.evictions += 1;
        self.touch(page_index);
        Ok(page_index)
    }

    fn touch(&mut self, page: usize) {
        self.clock += 1;
        self.pages[page].last_used = self.clock;
    }
}

fn can_fit(page: &AtlasPage, glyph: &AtlasBitmap, config: AtlasConfig) -> bool {
    let mut x = page.x;
    let mut y = page.y;
    let mut row_height = page.row_height;
    if x > 1 && x + glyph.width + 1 > config.page_width {
        x = 1;
        y += row_height + 1;
        row_height = 0;
    }
    let _ = x;
    y + row_height.max(glyph.height) < config.page_height
}

fn prepare_row(page: &mut AtlasPage, glyph_width: usize, page_width: usize) {
    if page.x > 1 && page.x + glyph_width + 1 > page_width {
        page.x = 1;
        page.y += page.row_height + 1;
        page.row_height = 0;
    }
}

thread_local! {
    static ATLASES: RefCell<BTreeMap<u32, AtlasSession>> = RefCell::new(BTreeMap::new());
    static NEXT_HANDLE: RefCell<u32> = const { RefCell::new(1) };
}

pub fn create(config: AtlasConfig) -> Result<(u32, AtlasStats), String> {
    let session = AtlasSession::new(config).map_err(|error| error.to_string())?;
    let handle = NEXT_HANDLE.with(|next| {
        let mut next = next.borrow_mut();
        let value = *next;
        *next = next.checked_add(1).unwrap_or(1);
        value
    });
    let stats = session.stats();
    ATLASES.with(|atlases| atlases.borrow_mut().insert(handle, session));
    Ok((handle, stats))
}

pub fn with_session<T>(
    handle: u32,
    operation: impl FnOnce(&mut AtlasSession) -> Result<T, String>,
) -> Result<T, String> {
    ATLASES.with(|atlases| {
        let mut atlases = atlases.borrow_mut();
        let session = atlases
            .get_mut(&handle)
            .ok_or_else(|| format!("unknown atlas handle {handle}"))?;
        operation(session)
    })
}

pub fn resolve(handle: u32, request: AtlasResolveRequest) -> Result<AtlasResolveResponse, String> {
    with_session(handle, |session| {
        let mut keys = request.keys;
        for record in request.records {
            let key = record.key;
            let placement = if let Some(existing) = session.placement(&key) {
                existing
            } else {
                let pixels = base64::engine::general_purpose::STANDARD
                    .decode(record.pixels_base64)
                    .map_err(|error| format!("invalid atlas R8 payload for {key}: {error}"))?;
                session
                    .put(AtlasBitmap {
                        key: key.clone(),
                        width: record.width,
                        height: record.height,
                        pixels,
                    })
                    .map_err(|error| error.to_string())?
            };
            let _ = placement;
            keys.push(key);
        }
        keys.sort();
        keys.dedup();
        let mut placed_keys = Vec::new();
        let mut placements = Vec::new();
        let mut missing_keys = Vec::new();
        for key in keys {
            if let Some(placement) = session.placement(&key) {
                placed_keys.push(key.clone());
                placements.push(AtlasResolvedGlyph { key, placement });
            } else {
                missing_keys.push(key);
            }
        }
        let lease = if placed_keys.is_empty() {
            None
        } else {
            Some(
                session
                    .acquire(&placed_keys)
                    .map_err(|error| error.to_string())?,
            )
        };
        Ok(AtlasResolveResponse {
            lease,
            placements,
            missing_keys,
            stats: session.stats(),
        })
    })
}

pub fn pages_since(
    handle: u32,
    request: AtlasPagesRequest,
) -> Result<Vec<AtlasPageUpdateMeta>, String> {
    let revisions = request
        .revisions
        .into_iter()
        .map(|entry| (entry.page, entry.revision))
        .collect();
    with_session(handle, |session| Ok(session.pages_since(&revisions)))
}

pub fn release(handle: u32, lease: u32) -> Result<bool, String> {
    with_session(handle, |session| Ok(session.release(lease)))
}

pub fn page_pixels(handle: u32, page: usize) -> Result<(*const u8, usize), String> {
    with_session(handle, |session| {
        let pixels = session
            .page_pixels(page)
            .ok_or_else(|| format!("unknown atlas page {page}"))?;
        Ok((pixels.as_ptr(), pixels.len()))
    })
}

pub fn destroy(handle: u32) -> bool {
    ATLASES.with(|atlases| atlases.borrow_mut().remove(&handle).is_some())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{AtlasBitmap, AtlasConfig, AtlasError, AtlasSession, PixelRect};

    fn glyph(key: &str, value: u8) -> AtlasBitmap {
        AtlasBitmap {
            key: key.to_string(),
            width: 5,
            height: 5,
            pixels: vec![value; 25],
        }
    }

    #[test]
    fn placement_and_dirty_rects_are_owned_by_the_atlas_session() {
        let mut atlas = AtlasSession::new(AtlasConfig {
            page_width: 8,
            page_height: 8,
            soft_pages: 1,
            hard_pages: 2,
        })
        .unwrap();
        let placement = atlas.put(glyph("alpha", 200)).unwrap();
        assert_eq!(placement.page, 0);
        assert_eq!((placement.pixel_rect.x, placement.pixel_rect.y), (1, 1));
        assert_eq!((placement.u0, placement.v0), (0.125, 0.125));
        let updates = atlas.pages_since(&Default::default());
        assert_eq!(updates.len(), 1);
        assert!(updates[0].full_upload);
        assert_eq!(atlas.page_pixels(0).unwrap().len(), 64);

        let mut dirty = AtlasSession::new(AtlasConfig {
            page_width: 8,
            page_height: 8,
            soft_pages: 1,
            hard_pages: 1,
        })
        .unwrap();
        dirty
            .put(AtlasBitmap {
                key: "first".to_string(),
                width: 2,
                height: 2,
                pixels: vec![7; 4],
            })
            .unwrap();
        let known = dirty.pages_since(&Default::default())[0].revision;
        dirty
            .put(AtlasBitmap {
                key: "second".to_string(),
                width: 1,
                height: 1,
                pixels: vec![9],
            })
            .unwrap();
        let updates = dirty.pages_since(&BTreeMap::from([(0, known)]));
        assert_eq!(updates.len(), 1);
        assert!(!updates[0].full_upload);
        assert_eq!(
            updates[0].dirty_rects,
            vec![PixelRect {
                x: 4,
                y: 1,
                width: 1,
                height: 1
            }]
        );
    }

    #[test]
    fn pinned_pages_fail_closed_and_recycle_only_after_release() {
        let mut atlas = AtlasSession::new(AtlasConfig {
            page_width: 8,
            page_height: 8,
            soft_pages: 1,
            hard_pages: 2,
        })
        .unwrap();
        let first = atlas.put(glyph("alpha", 10)).unwrap();
        let lease_a = atlas.acquire(&["alpha".to_string()]).unwrap();
        atlas.put(glyph("beta", 20)).unwrap();
        let lease_b = atlas.acquire(&["beta".to_string()]).unwrap();
        assert!(matches!(
            atlas.put(glyph("gamma", 30)),
            Err(AtlasError::MemoryBudgetExceeded { .. })
        ));
        assert!(atlas.release(lease_a));
        assert!(!atlas.release(lease_a));
        let recycled = atlas.put(glyph("gamma", 30)).unwrap();
        assert_eq!(recycled.page, first.page);
        assert_ne!(recycled.page_epoch, first.page_epoch);
        assert!(atlas.release(lease_b));
    }
}
