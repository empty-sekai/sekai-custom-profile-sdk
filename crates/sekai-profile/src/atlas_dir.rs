//! On-disk layout of the atlases a renderer installs.
//!
//! ```text
//! <root>/text/<family>/manifest.json    one text atlas per font family
//! <root>/tiers/<name>/manifest.json     optional larger point-size atlases of those faces
//! <root>/shape/manifest.json            the shape atlas
//! <root>/fallback/                      generated fallback glyphs (see [`Self::fallback_dir`])
//! ```
//!
//! [`crate::prepare::prepare_atlas_dir`] writes this layout and
//! [`crate::renderer::CustomProfileRenderer::with_atlas_dir`] installs it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::sdf::atlas::MappedSdfAtlas;
use crate::sdf::shape_atlas::MappedShapeSdfAtlas;

const TEXT_DIR: &str = "text";
const TIERS_DIR: &str = "tiers";
const SHAPE_DIR: &str = "shape";
const FALLBACK_DIR: &str = "fallback";
const MANIFEST_FILE: &str = "manifest.json";

/// A directory of atlases in the layout this module describes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AtlasDir {
    root: PathBuf,
}

/// Atlases opened from an [`AtlasDir`].
#[derive(Default)]
pub struct InstalledAtlases {
    /// One base atlas per font family, in directory order.
    pub text: Vec<Arc<MappedSdfAtlas>>,
    /// Atlases of the same faces at larger point sizes.
    pub tiers: Vec<Arc<MappedSdfAtlas>>,
    pub shape: Option<Arc<MappedShapeSdfAtlas>>,
}

impl AtlasDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory of the text atlas of `family`. Characters other than ASCII
    /// letters, digits, `.`, `_` and `-` become `_` in the directory name;
    /// the family itself is recorded in the atlas manifest.
    pub fn text_dir(&self, family: &str) -> PathBuf {
        let name: String = family
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        self.root.join(TEXT_DIR).join(name)
    }

    pub fn shape_dir(&self) -> PathBuf {
        self.root.join(SHAPE_DIR)
    }

    /// Directory for a [`crate::sdf::fallback_cache::PersistentFallbackSdfCache`]
    /// that keeps glyphs generated from a fallback face across runs.
    pub fn fallback_dir(&self) -> PathBuf {
        self.root.join(FALLBACK_DIR)
    }

    /// Manifests of the text atlases, in path order.
    pub fn text_manifests(&self) -> Result<Vec<PathBuf>, String> {
        manifests_under(&self.root.join(TEXT_DIR))
    }

    /// Manifests of the tier atlases, in path order.
    pub fn tier_manifests(&self) -> Result<Vec<PathBuf>, String> {
        manifests_under(&self.root.join(TIERS_DIR))
    }

    /// Opens every atlas in the directory.
    ///
    /// Text atlas pages are checked against the hashes their manifests record.
    /// Tier atlases are treated as immutable installs: their manifests, page
    /// sizes and headers are validated but their pages are not re-hashed. The
    /// font file of every text atlas's family must be installed and identical
    /// to the one the atlas was built from, because glyphs too large for any
    /// atlas are generated from that font while rendering. A tier must extend
    /// a base atlas of the same face at a smaller point size.
    pub fn open(&self) -> Result<InstalledAtlases, String> {
        let mut installed = InstalledAtlases::default();
        for manifest in self.text_manifests()? {
            let atlas = MappedSdfAtlas::open(&manifest)
                .map_err(|error| format!("{}: {error}", manifest.display()))?;
            check_font(&atlas)?;
            installed.text.push(Arc::new(atlas));
        }
        for manifest in self.tier_manifests()? {
            let atlas = MappedSdfAtlas::open_trusted_immutable_artifact(&manifest)
                .map_err(|error| format!("{}: {error}", manifest.display()))?;
            let tier = atlas.manifest();
            let extends_a_base = installed.text.iter().any(|base| {
                let base = base.manifest();
                base.font_family == tier.font_family
                    && base.font_sha256 == tier.font_sha256
                    && base.point_size < tier.point_size
            });
            if !extends_a_base {
                return Err(format!(
                    "{} does not extend a base text atlas of its face",
                    manifest.display()
                ));
            }
            installed.tiers.push(Arc::new(atlas));
        }
        let shape = self.shape_dir().join(MANIFEST_FILE);
        if shape.is_file() {
            let atlas = MappedShapeSdfAtlas::open(&shape)
                .map_err(|error| format!("{}: {error}", shape.display()))?;
            installed.shape = Some(Arc::new(atlas));
        }
        Ok(installed)
    }
}

fn check_font(atlas: &MappedSdfAtlas) -> Result<(), String> {
    let family = &atlas.manifest().font_family;
    let path = crate::sdf::outline::resolve_font_path(family)
        .ok_or_else(|| format!("no font file is installed for atlas family {family}"))?;
    let bytes =
        std::fs::read(&path).map_err(|error| format!("read {} failed: {error}", path.display()))?;
    if hex::encode(Sha256::digest(&bytes)) != atlas.manifest().font_sha256 {
        return Err(format!(
            "{} differs from the font the {family} atlas was built from",
            path.display()
        ));
    }
    Ok(())
}

fn manifests_under(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", dir.display())),
    };
    let mut manifests = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        let manifest = path.join(MANIFEST_FILE);
        if path.is_dir() && manifest.is_file() {
            manifests.push(manifest);
        }
    }
    manifests.sort();
    Ok(manifests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_names_map_to_portable_directory_names() {
        let dir = AtlasDir::new("atlases");
        assert_eq!(
            dir.text_dir("FOT-Yuruka Std UB"),
            Path::new("atlases").join("text").join("FOT-Yuruka_Std_UB")
        );
        assert_eq!(
            dir.text_dir("FOT-RodinNTLGPro-DB"),
            Path::new("atlases")
                .join("text")
                .join("FOT-RodinNTLGPro-DB")
        );
    }

    #[test]
    fn an_absent_directory_holds_no_atlas() {
        let root = tempfile::tempdir().expect("root");
        let installed = AtlasDir::new(root.path().join("missing"))
            .open()
            .expect("open");
        assert!(installed.text.is_empty() && installed.tiers.is_empty());
        assert!(installed.shape.is_none());
    }
}
