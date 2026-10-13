//! Atlas texel sources for the tile plan: installed Text atlases, the mixed Text and
//! Shape address space, and request-local Text pages.

use super::SdfTileError;
use crate::sdf::atlas::{MappedSdfAtlas, MappedSdfAtlasSet, SdfAtlasGlyphManifest};
use crate::sdf::shape::ShapeSdfTexel;
use crate::sdf::shape_atlas::MappedShapeSdfAtlas;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdfAtlasTexel {
    Text(u8),
    Shape(ShapeSdfTexel),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdfSwizzledFormat {
    TextR8,
    ShapeRg8,
}

#[derive(Clone, Copy, Debug)]
pub struct SdfSwizzledPage<'a> {
    pub width: u32,
    pub height: u32,
    pub format: SdfSwizzledFormat,
    pub payload: &'a [u8],
}

pub trait SdfAtlasSource {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)>;
    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel>;

    fn swizzled_page(&self, _atlas_set: u16, _page: u16) -> Option<SdfSwizzledPage<'_>> {
        None
    }
}

impl SdfAtlasSource for MappedSdfAtlas {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        if atlas_set != 0 {
            return None;
        }
        self.pages()
            .get(usize::from(page))
            .map(|page| (page.width(), page.height()))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if atlas_set != 0 {
            return None;
        }
        self.pages()
            .get(usize::from(page))?
            .texel(x, y)
            .map(SdfAtlasTexel::Text)
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        if atlas_set != 0 {
            return None;
        }
        let page = self.pages().get(usize::from(page))?;
        Some(SdfSwizzledPage {
            width: page.width(),
            height: page.height(),
            format: SdfSwizzledFormat::TextR8,
            payload: page.swizzled_payload(),
        })
    }
}

impl SdfAtlasSource for MappedSdfAtlasSet {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        self.atlas(atlas_set)?
            .pages()
            .get(usize::from(page))
            .map(|page| (page.width(), page.height()))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        self.atlas(atlas_set)?
            .pages()
            .get(usize::from(page))?
            .texel(x, y)
            .map(SdfAtlasTexel::Text)
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        let page = self.atlas(atlas_set)?.pages().get(usize::from(page))?;
        Some(SdfSwizzledPage {
            width: page.width(),
            height: page.height(),
            format: SdfSwizzledFormat::TextR8,
            payload: page.swizzled_payload(),
        })
    }
}

/// Presents the immutable Text and Shape atlas families as one typed address
/// space without widening Text texels to RG8. Shape receives the first atlas
/// set id after all installed Text families.
pub struct MixedSdfAtlasSource<'a> {
    text: &'a MappedSdfAtlasSet,
    shape: Option<&'a MappedShapeSdfAtlas>,
    shape_atlas_set: Option<u16>,
    runtime_text: &'a [RuntimeTextSdfPage],
    runtime_text_atlas_set: Option<u16>,
}

/// Request-local high-resolution Text SDF page. It is never inserted into the
/// immutable atlas registry or any process cache.
#[derive(Clone)]
pub(crate) struct RuntimeTextSdfPage {
    width: u32,
    height: u32,
    payload: Vec<u8>,
}

impl RuntimeTextSdfPage {
    pub(crate) fn resident_bytes(&self) -> u64 {
        self.payload.len() as u64
    }

    pub(crate) fn from_outline(
        glyph: &crate::sdf::outline::OutlineSdfGlyph,
        codepoint: u32,
        page: u16,
    ) -> Result<(Self, SdfAtlasGlyphManifest), SdfTileError> {
        let width = glyph
            .width()
            .checked_add(7)
            .ok_or(SdfTileError::SizeOverflow)?
            / 8
            * 8;
        let height = glyph
            .height()
            .checked_add(7)
            .ok_or(SdfTileError::SizeOverflow)?
            / 8
            * 8;
        let mut linear = vec![
            0u8;
            width
                .checked_mul(height)
                .ok_or(SdfTileError::SizeOverflow)?
        ];
        for y in 0..glyph.height() {
            linear[y * width..y * width + glyph.width()]
                .copy_from_slice(&glyph.pixels()[y * glyph.width()..(y + 1) * glyph.width()]);
        }
        let mut payload = vec![0u8; linear.len()];
        let blocks_per_row = width / 8;
        for y in 0..height {
            for x in 0..width {
                let block = (y / 8) * blocks_per_row + x / 8;
                let in_block = (y % 8) * 8 + x % 8;
                payload[block * 64 + in_block] = linear[y * width + x];
            }
        }
        Ok((
            Self {
                width: u32::try_from(width).map_err(|_| SdfTileError::SizeOverflow)?,
                height: u32::try_from(height).map_err(|_| SdfTileError::SizeOverflow)?,
                payload,
            },
            SdfAtlasGlyphManifest {
                codepoint,
                page,
                rect: [0, 0, glyph.width() as u32, glyph.height() as u32],
                plane_bearing: [glyph.plane_bearing_x(), glyph.plane_bearing_y()],
                plane_size: [glyph.plane_width(), glyph.plane_height()],
                plane_advance_x: glyph.plane_advance_x(),
            },
        ))
    }
}

impl<'a> MixedSdfAtlasSource<'a> {
    pub fn new(
        text: &'a MappedSdfAtlasSet,
        shape: Option<&'a MappedShapeSdfAtlas>,
    ) -> Result<Self, SdfTileError> {
        let shape_atlas_set = shape
            .map(|_| u16::try_from(text.len()).map_err(|_| SdfTileError::TooManyAtlasSets))
            .transpose()?;
        Ok(Self {
            text,
            shape,
            shape_atlas_set,
            runtime_text: &[],
            runtime_text_atlas_set: None,
        })
    }

    pub(crate) fn with_runtime_text(
        text: &'a MappedSdfAtlasSet,
        shape: Option<&'a MappedShapeSdfAtlas>,
        runtime_text: &'a [RuntimeTextSdfPage],
    ) -> Result<Self, SdfTileError> {
        let mut source = Self::new(text, shape)?;
        if !runtime_text.is_empty() {
            source.runtime_text_atlas_set = Some(
                u16::try_from(text.len() + usize::from(shape.is_some()))
                    .map_err(|_| SdfTileError::TooManyAtlasSets)?,
            );
            source.runtime_text = runtime_text;
        }
        Ok(source)
    }

    pub const fn shape_atlas_set(&self) -> Option<u16> {
        self.shape_atlas_set
    }

    pub const fn runtime_text_atlas_set(&self) -> Option<u16> {
        self.runtime_text_atlas_set
    }
}

impl SdfAtlasSource for MixedSdfAtlasSource<'_> {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        if Some(atlas_set) == self.runtime_text_atlas_set {
            let page = self.runtime_text.get(usize::from(page))?;
            return Some((page.width, page.height));
        }
        if Some(atlas_set) == self.shape_atlas_set {
            return self
                .shape?
                .pages()
                .get(usize::from(page))
                .map(|page| (page.width(), page.height()));
        }
        self.text
            .atlas(atlas_set)?
            .pages()
            .get(usize::from(page))
            .map(|page| (page.width(), page.height()))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if Some(atlas_set) == self.runtime_text_atlas_set {
            let page = self.runtime_text.get(usize::from(page))?;
            if x >= page.width || y >= page.height {
                return None;
            }
            let blocks_per_row = page.width / 8;
            let block = (y / 8) * blocks_per_row + x / 8;
            let in_block = (y % 8) * 8 + x % 8;
            return page
                .payload
                .get((block * 64 + in_block) as usize)
                .copied()
                .map(SdfAtlasTexel::Text);
        }
        if Some(atlas_set) == self.shape_atlas_set {
            return self
                .shape?
                .pages()
                .get(usize::from(page))?
                .texel(x, y)
                .map(SdfAtlasTexel::Shape);
        }
        self.text
            .atlas(atlas_set)?
            .pages()
            .get(usize::from(page))?
            .texel(x, y)
            .map(SdfAtlasTexel::Text)
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        if Some(atlas_set) == self.runtime_text_atlas_set {
            let page = self.runtime_text.get(usize::from(page))?;
            return Some(SdfSwizzledPage {
                width: page.width,
                height: page.height,
                format: SdfSwizzledFormat::TextR8,
                payload: &page.payload,
            });
        }
        if Some(atlas_set) == self.shape_atlas_set {
            let page = self.shape?.pages().get(usize::from(page))?;
            return Some(SdfSwizzledPage {
                width: page.width(),
                height: page.height(),
                format: SdfSwizzledFormat::ShapeRg8,
                payload: page.swizzled_payload(),
            });
        }
        let page = self.text.atlas(atlas_set)?.pages().get(usize::from(page))?;
        Some(SdfSwizzledPage {
            width: page.width(),
            height: page.height(),
            format: SdfSwizzledFormat::TextR8,
            payload: page.swizzled_payload(),
        })
    }
}
