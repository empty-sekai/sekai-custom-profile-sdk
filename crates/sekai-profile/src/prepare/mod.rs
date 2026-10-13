//! Building the SDF atlases the native renderer samples.
//!
//! [`prepare_atlas_dir`] fills an [`AtlasDir`] with a text atlas for every
//! font family the master data names and a shape atlas for every shape
//! sprite. [`build_text_atlas`] and [`build_shape_atlas`] build single atlases.

pub mod shape;
pub mod text;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use sha2::{Digest, Sha256};

pub use shape::{build_shape_atlas, ShapeAtlasOptions, ShapeResource};
pub use text::{build_text_atlas, font_codepoints, TextAtlasOptions};

use crate::atlas_dir::AtlasDir;
use crate::masterdata::JsonMasterDataProvider;
use crate::sdf::atlas::MappedSdfAtlas;
use crate::sdf::shape_atlas::MappedShapeSdfAtlas;

pub(crate) const MANIFEST_FILE: &str = "manifest.json";

/// What [`prepare_atlas_dir`] builds.
#[derive(Clone, Debug, Default)]
pub struct PrepareOptions {
    /// Codepoints to cover, per font family. `None` covers every codepoint
    /// each family's font maps; [`crate::renderer::CustomProfileRenderer::text_codepoints`]
    /// gives the codepoints particular cards draw.
    pub codepoints: Option<BTreeMap<String, BTreeSet<u32>>>,
    /// Rebuild atlases that already cover the request.
    pub force: bool,
}

/// What [`prepare_atlas_dir`] did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrepareReport {
    /// Text atlases by font family.
    pub text: BTreeMap<String, Prepared>,
    pub shape: Option<Prepared>,
    /// Font families with no installed font file. Cards that use them cannot
    /// be rendered until the font is installed and the atlas built.
    pub missing_fonts: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Prepared {
    /// The atlas was written by this call.
    Built,
    /// An existing atlas already covered the request and was kept.
    Kept,
}

/// Builds the atlases `masterdata` calls for into `dir`.
///
/// Text atlases are built for the families of `customProfileTextFonts`, or
/// for the families in [`PrepareOptions::codepoints`] when it is set; the
/// font files are found through the configured font directories. An existing
/// text atlas is kept when it was built from the installed font and already
/// holds the requested codepoints; otherwise it is rebuilt to cover both its
/// previous glyphs and the request. The shape atlas is built from
/// `customProfileShapeResources`, reading each sprite through `shape_source`,
/// unless one already exists. Every atlas is written to a temporary directory
/// first and moved into place once complete.
pub fn prepare_atlas_dir(
    dir: &AtlasDir,
    masterdata: &JsonMasterDataProvider,
    shape_source: impl FnMut(&ShapeResource) -> Result<Vec<u8>, String>,
    options: &PrepareOptions,
) -> Result<PrepareReport, String> {
    let mut report = PrepareReport::default();
    let families: Vec<String> = match &options.codepoints {
        Some(demand) => demand.keys().cloned().collect(),
        None => masterdata.text_font_families(),
    };
    for family in families {
        let Some(font_path) = crate::sdf::outline::resolve_font_path(&family) else {
            report.missing_fonts.push(family);
            continue;
        };
        let requested = options
            .codepoints
            .as_ref()
            .and_then(|demand| demand.get(&family));
        let prepared = prepare_text_atlas(dir, &family, &font_path, requested, options.force)?;
        report.text.insert(family, prepared);
    }

    let shape_manifest = dir.shape_dir().join(MANIFEST_FILE);
    let keep_shape = !options.force && MappedShapeSdfAtlas::open(&shape_manifest).is_ok();
    if keep_shape {
        report.shape = Some(Prepared::Kept);
    } else if let Some(table) = masterdata.table("customProfileShapeResources") {
        let resources = table
            .all()
            .iter()
            .map(|row| serde_json::from_value::<ShapeResource>(row.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("customProfileShapeResources: {error}"))?;
        if !resources.is_empty() {
            replace_dir(&dir.shape_dir(), |staging| {
                build_shape_atlas(&resources, shape_source, &ShapeAtlasOptions::new(staging))
                    .map(|_| ())
            })?;
            report.shape = Some(Prepared::Built);
        }
    }
    Ok(report)
}

fn prepare_text_atlas(
    dir: &AtlasDir,
    family: &str,
    font_path: &Path,
    requested: Option<&BTreeSet<u32>>,
    force: bool,
) -> Result<Prepared, String> {
    let target = dir.text_dir(family);
    let existing = MappedSdfAtlas::open(target.join(MANIFEST_FILE)).ok();
    let font_bytes = std::fs::read(font_path)
        .map_err(|error| format!("read {} failed: {error}", font_path.display()))?;
    let font_sha256 = hex::encode(Sha256::digest(&font_bytes));
    let existing = existing.filter(|atlas| {
        atlas.manifest().font_family == family && atlas.manifest().font_sha256 == font_sha256
    });

    let codepoints = match requested {
        None => {
            let full = existing.as_ref().is_some_and(|atlas| {
                let generation = &atlas.manifest().generation;
                generation.requested_codepoint_count == generation.cmap_codepoint_count
            });
            if full && !force {
                return Ok(Prepared::Kept);
            }
            None
        }
        Some(requested) => {
            let mapped = font_codepoints(&font_bytes)?;
            let wanted: BTreeSet<u32> = requested.intersection(&mapped).copied().collect();
            let held: BTreeSet<u32> = existing
                .as_ref()
                .map(|atlas| {
                    atlas
                        .manifest()
                        .glyphs
                        .iter()
                        .map(|glyph| glyph.codepoint)
                        .collect()
                })
                .unwrap_or_default();
            if existing.is_some() && !force && wanted.is_subset(&held) {
                return Ok(Prepared::Kept);
            }
            let union: BTreeSet<u32> = wanted.union(&held).copied().collect();
            if union.is_empty() {
                // The font maps none of the requested characters. The family
                // still needs an atlas, so it holds the missing-glyph square,
                // or the first character the font maps when it has no square.
                let square =
                    u32::from(sekai_profile_core::tmp_text::glyph::MISSING_GLYPH_CHARACTER);
                let only = if mapped.contains(&square) {
                    Some(square)
                } else {
                    mapped.first().copied()
                };
                let only = only.ok_or_else(|| format!("the font of {family} maps no character"))?;
                Some(BTreeSet::from([only]))
            } else {
                Some(union)
            }
        }
    };
    let mut options = TextAtlasOptions::new(family, Path::new(""));
    options.codepoints = codepoints;
    replace_dir(&target, move |staging| {
        options.output = staging.to_path_buf();
        build_text_atlas(&options).map(|_| ())
    })?;
    Ok(Prepared::Built)
}

/// Runs `build` against an empty sibling directory of `target`, then replaces
/// `target` with it. `target` is left untouched when `build` fails.
fn replace_dir(
    target: &Path,
    build: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let name = target
        .file_name()
        .ok_or_else(|| format!("{} has no directory name", target.display()))?
        .to_string_lossy()
        .into_owned();
    let staging = target.with_file_name(format!(".{name}.partial"));
    remove_dir_if_present(&staging)?;
    let result = build(&staging);
    if let Err(error) = result {
        remove_dir_if_present(&staging).ok();
        return Err(error);
    }
    remove_dir_if_present(target)?;
    std::fs::rename(&staging, target).map_err(|error| {
        format!(
            "move {} to {} failed: {error}",
            staging.display(),
            target.display()
        )
    })
}

fn remove_dir_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove {} failed: {error}", path.display())),
    }
}

/// Creates `path`, or checks that it is an empty directory.
pub(crate) fn ensure_empty_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        let mut entries = std::fs::read_dir(path)
            .map_err(|error| format!("read {} failed: {error}", path.display()))?;
        if entries.next().is_some() {
            return Err(format!("{} is not empty", path.display()));
        }
        Ok(())
    } else {
        std::fs::create_dir_all(path)
            .map_err(|error| format!("create {} failed: {error}", path.display()))
    }
}

pub(crate) fn write_manifest(path: &Path, manifest: &impl serde::Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("serialize manifest failed: {error}"))?;
    bytes.push(b'\n');
    std::fs::write(path, bytes).map_err(|error| format!("write {} failed: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape_masterdata() -> JsonMasterDataProvider {
        let mut provider = JsonMasterDataProvider::empty();
        provider
            .insert_table(
                "customProfileShapeResources",
                r#"[{"id":1,"fileName":"square","resourceLoadVal":"custom_profile/shape"}]"#,
            )
            .expect("table");
        provider
    }

    fn square_png() -> Vec<u8> {
        crate::codec::png::encode_rgba(1, 1, &[255, 0, 0, 255]).expect("encode")
    }

    #[test]
    fn builds_the_shape_atlas_once_and_keeps_it_afterwards() {
        let root = tempfile::tempdir().expect("root");
        let dir = AtlasDir::new(root.path());
        let masterdata = shape_masterdata();
        let options = PrepareOptions {
            codepoints: Some(BTreeMap::new()),
            force: false,
        };
        let mut reads = 0;
        let first = prepare_atlas_dir(
            &dir,
            &masterdata,
            |_| {
                reads += 1;
                Ok(square_png())
            },
            &options,
        )
        .expect("first prepare");
        assert_eq!(first.shape, Some(Prepared::Built));
        assert_eq!(reads, 1);
        let installed = dir.open().expect("open");
        assert_eq!(installed.shape.expect("shape").manifest().shapes.len(), 1);

        let second = prepare_atlas_dir(&dir, &masterdata, |_| Ok(square_png()), &options)
            .expect("second prepare");
        assert_eq!(second.shape, Some(Prepared::Kept));
    }

    #[test]
    fn a_failed_build_leaves_the_previous_atlas_in_place() {
        let root = tempfile::tempdir().expect("root");
        let dir = AtlasDir::new(root.path());
        let masterdata = shape_masterdata();
        let keep = PrepareOptions {
            codepoints: Some(BTreeMap::new()),
            force: false,
        };
        prepare_atlas_dir(&dir, &masterdata, |_| Ok(square_png()), &keep).expect("prepare");
        let force = PrepareOptions {
            force: true,
            ..keep
        };
        let result = prepare_atlas_dir(&dir, &masterdata, |_| Err("unreadable".into()), &force);
        assert!(result.is_err());
        assert!(dir.open().expect("open").shape.is_some());
    }

    #[test]
    fn families_without_a_font_file_are_reported() {
        let root = tempfile::tempdir().expect("root");
        let demand = BTreeMap::from([(
            "Family Without A Font File".to_string(),
            BTreeSet::from([u32::from('A')]),
        )]);
        let report = prepare_atlas_dir(
            &AtlasDir::new(root.path()),
            &JsonMasterDataProvider::empty(),
            |_| Err("unused".into()),
            &PrepareOptions {
                codepoints: Some(demand),
                force: false,
            },
        )
        .expect("prepare");
        assert_eq!(report.missing_fonts, ["Family Without A Font File"]);
        assert!(report.text.is_empty());
        assert_eq!(report.shape, None);
    }

    #[test]
    fn a_text_atlas_grows_to_cover_new_codepoints_and_keeps_old_ones() {
        if crate::sdf::outline::resolve_font_path("DejaVu Sans").is_none() {
            eprintln!("skipping: DejaVu Sans is not installed");
            return;
        }
        let root = tempfile::tempdir().expect("root");
        let dir = AtlasDir::new(root.path());
        let request = |text: &str| PrepareOptions {
            codepoints: Some(BTreeMap::from([(
                "DejaVu Sans".to_string(),
                text.chars().map(u32::from).collect(),
            )])),
            force: false,
        };
        let empty = JsonMasterDataProvider::empty();
        let unused = |_: &ShapeResource| Err("unused".to_string());
        let first = prepare_atlas_dir(&dir, &empty, unused, &request("AB")).expect("first");
        assert_eq!(first.text["DejaVu Sans"], Prepared::Built);
        let kept = prepare_atlas_dir(&dir, &empty, unused, &request("A")).expect("kept");
        assert_eq!(kept.text["DejaVu Sans"], Prepared::Kept);
        let grown = prepare_atlas_dir(&dir, &empty, unused, &request("C")).expect("grown");
        assert_eq!(grown.text["DejaVu Sans"], Prepared::Built);
        let atlas = MappedSdfAtlas::open(dir.text_dir("DejaVu Sans").join(MANIFEST_FILE))
            .expect("open atlas");
        for ch in ['A', 'B', 'C'] {
            assert!(atlas.glyph(u32::from(ch)).is_some(), "{ch}");
        }
    }
}
