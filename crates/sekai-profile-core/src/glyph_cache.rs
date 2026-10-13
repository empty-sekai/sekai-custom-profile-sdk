//! Session glyph cache keyed by font, raster contract and glyph index.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GlyphCacheKey {
    pub region: String,
    pub font_digest: [u8; 32],
    pub font_engine_fingerprint: String,
    pub raster_contract: String,
    pub glyph_index: u32,
}

#[derive(Clone, Debug)]
pub struct GlyphCacheEntry {
    pub metrics: [i32; 6],
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlyphCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub generations: u64,
    pub bytes: usize,
}

#[derive(Default)]
pub struct SessionGlyphCache {
    entries: HashMap<GlyphCacheKey, GlyphCacheEntry>,
    stats: GlyphCacheStats,
}

impl SessionGlyphCache {
    pub fn get_or_insert_with<F>(&mut self, key: GlyphCacheKey, generate: F) -> &GlyphCacheEntry
    where
        F: FnOnce() -> GlyphCacheEntry,
    {
        if self.entries.contains_key(&key) {
            self.stats.hits += 1;
        } else {
            self.stats.misses += 1;
            self.stats.generations += 1;
            let entry = generate();
            self.stats.bytes += entry.pixels.len();
            self.entries.insert(key.clone(), entry);
        }
        &self.entries[&key]
    }

    pub fn stats(&self) -> &GlyphCacheStats {
        &self.stats
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.stats.bytes = 0;
    }
}
