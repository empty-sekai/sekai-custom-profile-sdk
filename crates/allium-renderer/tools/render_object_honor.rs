use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use allium_renderer::assets::AssetStore;
use allium_renderer::masterdata::{
    MasterData, MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo,
};
use allium_renderer::render_object::{
    bonds_honor_object_key, standard_honor_object_key, MappedRenderObjectStore, RenderObjectKind,
    RenderObjectStoreWriter, RenderObjectWrite, HONOR_RENDER_OBJECT_CONTRACT,
};
use allium_renderer::types::{
    BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry, HonorGroupEntry,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skia_safe::{AlphaType, Color, ColorType, ImageInfo};

const HONOR_PLAN_SCHEMA: &str = "allium.honor-render-object-plan.v1";
const HONOR_BUILD_SCHEMA: &str = "allium.honor-render-object-build.v1";
const CURRENT_CN_6_0_0_43_OBJECT_COUNT: u64 = 87_498;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BondsPlanEntry {
    id: i32,
    bonds_group_id: i32,
    game_character_unit_id1: i32,
    game_character_unit_id2: i32,
    honor_rarity: String,
    #[serde(default)]
    configurable_unit_virtual_singer: bool,
    #[serde(default)]
    levels: Vec<PlanLevel>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct PlanLevel {
    level: i32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameCharacterUnit {
    id: i32,
    game_character_id: i32,
    unit: String,
}

#[derive(Clone, Debug)]
enum HonorObjectRecipe {
    Standard {
        honor_id: i32,
        honor_level: i32,
        full_size: bool,
    },
    Bonds {
        honor_id: i32,
        honor_level: i32,
        full_size: bool,
        word_id: i64,
        inverse: bool,
        use_unit_virtual_singer: bool,
    },
}

#[derive(Clone, Debug)]
struct PlannedHonorObject {
    key: String,
    recipe: HonorObjectRecipe,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
struct HonorCoverage {
    standard_main: u64,
    standard_sub: u64,
    bonds_main: u64,
    bonds_sub: u64,
    total: u64,
    payload_bytes: u64,
}

struct HonorPlan {
    masterdata_identity: String,
    provider: Arc<HonorMasterDataProvider>,
    objects: Vec<PlannedHonorObject>,
    coverage: HonorCoverage,
}

pub(super) fn planned_honor_catalog_keys(
    masterdata_dir: &Path,
) -> Result<Vec<(String, &'static str)>, String> {
    let plan = HonorPlan::load(masterdata_dir)?;
    Ok(plan
        .objects
        .into_iter()
        .map(|object| {
            let kind = match object.recipe {
                HonorObjectRecipe::Standard { .. } => "standard_honor",
                HonorObjectRecipe::Bonds { .. } => "bonds_honor",
            };
            (object.key, kind)
        })
        .collect())
}

struct HonorMasterDataProvider {
    honors: BTreeMap<i32, HonorEntry>,
    honor_groups: BTreeMap<i32, HonorGroupEntry>,
    bonds: BTreeMap<i32, BondsPlanEntry>,
    words: BTreeMap<i64, BondsHonorWordEntry>,
    units: Vec<GameCharacterUnit>,
}

pub(super) fn run_honor_plan(args: &[String]) -> Result<(), String> {
    if !(3..=4).contains(&args.len()) {
        return Err(format!(
            "usage: {} --honor-plan <masterdata-dir> [expected-object-count]",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let started = Instant::now();
    let plan = HonorPlan::load(Path::new(&args[2]))?;
    let expected = parse_expected_count(args.get(3))?;
    enforce_expected_count(&plan.coverage, expected)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": HONOR_PLAN_SCHEMA,
            "masterdata_dir": Path::new(&args[2]),
            "masterdata_identity": plan.masterdata_identity,
            "honor_contract": HONOR_RENDER_OBJECT_CONTRACT,
            "coverage": plan.coverage,
            "expected_object_count": expected,
            "current_cn_6_0_0_43_reference": CURRENT_CN_6_0_0_43_OBJECT_COUNT,
            "matches_current_cn_6_0_0_43": plan.coverage.total == CURRENT_CN_6_0_0_43_OBJECT_COUNT,
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        }))
        .map_err(|error| format!("serialize Honor plan failed: {error}"))?
    );
    Ok(())
}

pub(super) fn run_honor_store(args: &[String]) -> Result<(), String> {
    run_honor_store_impl(args, None)
}

pub(super) fn run_honor_delta_store(args: &[String]) -> Result<(), String> {
    if !(10..=11).contains(&args.len()) {
        return Err(format!(
            "usage: {} --honor-delta-store <output-root> <masterdata-dir> <static-root> <asset-cache-root> <available-assets.json> <asset-identity> <expected-object-count> <base-manifest> [page-mib]",
            args.first().map(String::as_str).unwrap_or("build-render-object-store")
        ));
    }
    let available_assets = load_available_assets(Path::new(&args[6]))?;
    run_honor_store_impl(args, Some((PathBuf::from(&args[9]), available_assets)))
}

fn run_honor_store_impl(
    args: &[String],
    delta_inputs: Option<(PathBuf, BTreeSet<String>)>,
) -> Result<(), String> {
    let delta_mode = delta_inputs.is_some();
    if !(8..=9).contains(&args.len()) {
        if !delta_mode {
            return Err(format!(
                "usage: {} --honor-store <output-root> <masterdata-dir> <static-root> <asset-cache-root> <asset-identity> <expected-object-count> [page-mib]",
                args.first().map(String::as_str).unwrap_or("build-render-object-store")
            ));
        }
    }
    let started = Instant::now();
    let output_root = PathBuf::from(&args[2]);
    let masterdata_dir = PathBuf::from(&args[3]);
    let static_root = PathBuf::from(&args[4]);
    let asset_cache_root = PathBuf::from(&args[5]);
    let identity_index = if delta_mode { 7 } else { 6 };
    let asset_identity = args[identity_index].trim();
    if asset_identity.is_empty() {
        return Err("asset identity must not be empty".into());
    }
    let expected = parse_expected_count(args.get(identity_index + 1))?
        .ok_or_else(|| "expected object count is required for Honor publication".to_string())?;
    let page_mib = args
        .get(if delta_mode { 10 } else { 8 })
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|error| format!("invalid page-mib: {error}"))?
        .unwrap_or(512);
    let page_payload_limit = page_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| "page-mib overflow".to_string())?;

    let plan = HonorPlan::load(&masterdata_dir)?;
    enforce_expected_count(&plan.coverage, Some(expected))?;
    let base_manifest = delta_inputs.as_ref().map(|(path, _)| path);
    let available_assets = delta_inputs.as_ref().map(|(_, assets)| assets);
    let base = base_manifest
        .as_ref()
        .map(|path| {
            MappedRenderObjectStore::open_metadata_catalog(path)
                .map_err(|error| format!("open Honor delta base failed: {error}"))
        })
        .transpose()?;
    let base_keys = base
        .as_ref()
        .map(|store| {
            store
                .manifest()
                .objects
                .iter()
                .filter(|object| {
                    matches!(
                        object.kind,
                        RenderObjectKind::StandardHonor | RenderObjectKind::BondsHonor
                    )
                })
                .map(|object| object.key.as_str())
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();
    let objects = plan
        .objects
        .iter()
        .filter(|object| !base_keys.contains(object.key.as_str()))
        .collect::<Vec<_>>();
    if delta_mode && objects.is_empty() {
        let base = base.as_ref().expect("delta mode has base");
        print_build_report(
            true,
            &base.manifest().source_identity,
            base_manifest.expect("delta mode has path"),
            base,
            &plan.coverage,
            &[],
            started,
        )?;
        return Ok(());
    }
    let source_identity = if let Some(base) = &base {
        honor_source_identity(
            &plan.masterdata_identity,
            &format!("{asset_identity}:{}", base.manifest_sha256()),
        )
    } else {
        honor_source_identity(&plan.masterdata_identity, asset_identity)
    };
    std::fs::create_dir_all(&output_root)
        .map_err(|error| format!("create {} failed: {error}", output_root.display()))?;
    let final_dir = output_root.join(&source_identity);
    let final_manifest = final_dir.join("manifest.json");
    if final_manifest.is_file() {
        let catalog = MappedRenderObjectStore::open_metadata_catalog(&final_manifest)
            .map_err(|error| format!("open existing Honor generation failed: {error}"))?;
        if delta_mode {
            enforce_delta_keys(&catalog, &objects)?;
        } else {
            enforce_manifest_coverage(&catalog, &plan.coverage)?;
        }
        print_build_report(
            true,
            &source_identity,
            &final_manifest,
            &catalog,
            &plan.coverage,
            &[],
            started,
        )?;
        return Ok(());
    }
    if final_dir.exists() {
        return Err(format!(
            "Honor generation exists without a valid manifest: {}",
            final_dir.display()
        ));
    }

    let (assets, md) = load_honor_render_inputs(&plan, &static_root, &asset_cache_root)?;
    let missing_asset_keys = audit_missing_objects(&objects, &md, &assets)?;
    let audit_path = output_root.join("honor-missing-assets.json");
    write_asset_audit(&audit_path, &plan, &missing_asset_keys)?;
    let required_missing_asset_keys =
        required_missing_assets(&missing_asset_keys, available_assets);
    if !required_missing_asset_keys.is_empty() {
        let preview = required_missing_asset_keys
            .iter()
            .take(16)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Honor source audit found {} missing asset(s); preview: [{}]; repair plan: {}",
            required_missing_asset_keys.len(),
            preview,
            audit_path.display()
        ));
    }

    let staging_dir =
        output_root.join(format!(".staging-{source_identity}-{}", std::process::id()));
    if staging_dir.exists() {
        return Err(format!(
            "Honor staging directory already exists: {}",
            staging_dir.display()
        ));
    }

    let mut writer = RenderObjectStoreWriter::create(
        &staging_dir,
        format!("honor-final:{source_identity}"),
        page_payload_limit,
    )
    .map_err(|error| error.to_string())?;
    for (index, object) in objects.iter().enumerate() {
        let rendered = render_object(object, &md, &assets)?;
        let source_sha256 = object_source_identity(&source_identity, &object.key);
        writer
            .add(RenderObjectWrite {
                key: &object.key,
                kind: match object.recipe {
                    HonorObjectRecipe::Standard { .. } => RenderObjectKind::StandardHonor,
                    HonorObjectRecipe::Bonds { .. } => RenderObjectKind::BondsHonor,
                },
                source_sha256: &source_sha256,
                width: rendered.width,
                height: rendered.height,
                row_bytes: rendered.row_bytes,
                pixels: &rendered.pixels,
            })
            .map_err(|error| error.to_string())?;
        if (index + 1) % 1000 == 0 || index + 1 == objects.len() {
            eprintln!(
                "Honor render-object progress: {}/{}",
                index + 1,
                objects.len()
            );
        }
    }
    let staging_manifest = writer.finish().map_err(|error| error.to_string())?;
    let catalog = MappedRenderObjectStore::open_metadata_catalog(&staging_manifest)
        .map_err(|error| format!("verify staged Honor catalog failed: {error}"))?;
    if delta_mode {
        enforce_delta_keys(&catalog, &objects)?;
    } else {
        enforce_manifest_coverage(&catalog, &plan.coverage)?;
    }
    let missing_asset_keys = assets.take_missing_image_keys();
    let required_missing_asset_keys =
        required_missing_assets(&missing_asset_keys, available_assets);
    if !required_missing_asset_keys.is_empty() {
        return Err(format!(
            "Honor source set changed during generation; {} asset(s) became unavailable",
            required_missing_asset_keys.len()
        ));
    }
    drop(catalog);
    std::fs::rename(&staging_dir, &final_dir).map_err(|error| {
        format!(
            "publish Honor generation {} to {} failed: {error}",
            staging_dir.display(),
            final_dir.display()
        )
    })?;
    let catalog = MappedRenderObjectStore::open_metadata_catalog(&final_manifest)
        .map_err(|error| format!("open published Honor catalog failed: {error}"))?;
    print_build_report(
        false,
        &source_identity,
        &final_manifest,
        &catalog,
        &plan.coverage,
        &required_missing_asset_keys,
        started,
    )
}

fn load_available_assets(path: &Path) -> Result<BTreeSet<String>, String> {
    serde_json::from_slice(
        &std::fs::read(path).map_err(|error| format!("read {} failed: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {} failed: {error}", path.display()))
}

fn required_missing_assets(
    missing: &[String],
    available: Option<&BTreeSet<String>>,
) -> Vec<String> {
    match available {
        Some(available) => missing
            .iter()
            .filter(|key| available.contains(key.as_str()))
            .cloned()
            .collect(),
        None => missing.to_vec(),
    }
}

fn enforce_delta_keys(
    catalog: &MappedRenderObjectStore,
    expected: &[&PlannedHonorObject],
) -> Result<(), String> {
    let actual = catalog
        .manifest()
        .objects
        .iter()
        .map(|object| object.key.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if actual.len() != expected.len()
        || expected
            .iter()
            .any(|object| !actual.contains(object.key.as_str()))
    {
        return Err("Honor delta manifest does not cover every missing planned object".into());
    }
    Ok(())
}

pub(super) fn run_honor_assets_audit(args: &[String]) -> Result<(), String> {
    if args.len() != 6 {
        return Err(format!(
            "usage: {} --honor-assets-audit <masterdata-dir> <static-root> <asset-cache-root> <output.json>",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let started = Instant::now();
    let plan = HonorPlan::load(Path::new(&args[2]))?;
    let (assets, md) = load_honor_render_inputs(&plan, Path::new(&args[3]), Path::new(&args[4]))?;
    let missing = audit_missing_assets(&plan, &md, &assets)?;
    let output = PathBuf::from(&args[5]);
    write_asset_audit(&output, &plan, &missing)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "allium.honor-asset-audit-report.v1",
            "output": output,
            "planned_object_count": plan.coverage.total,
            "missing_asset_count": missing.len(),
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        }))
        .map_err(|error| format!("serialize Honor asset audit report failed: {error}"))?
    );
    Ok(())
}

fn load_honor_render_inputs(
    plan: &HonorPlan,
    static_root: &Path,
    asset_cache_root: &Path,
) -> Result<(AssetStore, MasterData), String> {
    let mut assets = AssetStore::new(1024);
    assets.set_disk_cache_dir(asset_cache_root.to_path_buf());
    assets.load_static_dir(static_root)?;
    let md = MasterData::new(plan.provider.clone());
    Ok((assets, md))
}

fn audit_missing_assets(
    plan: &HonorPlan,
    md: &MasterData,
    assets: &AssetStore,
) -> Result<Vec<String>, String> {
    let objects = plan.objects.iter().collect::<Vec<_>>();
    audit_missing_objects(&objects, md, assets)
}

fn audit_missing_objects(
    objects: &[&PlannedHonorObject],
    md: &MasterData,
    assets: &AssetStore,
) -> Result<Vec<String>, String> {
    for (index, object) in objects.iter().enumerate() {
        render_object(object, md, assets)?;
        if (index + 1) % 5000 == 0 || index + 1 == objects.len() {
            eprintln!(
                "Honor source audit progress: {}/{}",
                index + 1,
                objects.len()
            );
        }
    }
    Ok(assets.take_missing_image_keys())
}

fn write_asset_audit(output: &Path, plan: &HonorPlan, missing: &[String]) -> Result<(), String> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create {} failed: {error}", parent.display()))?;
    let entries = missing
        .iter()
        .map(|key| {
            serde_json::json!({
                "key": key,
                "logical_path": allium_renderer::asset_keys::key_to_s3_path(key, "assets/cn/"),
            })
        })
        .collect::<Vec<_>>();
    let document = serde_json::json!({
        "schema": "allium.honor-asset-repair.v1",
        "masterdata_identity": plan.masterdata_identity,
        "planned_object_count": plan.coverage.total,
        "missing_asset_count": entries.len(),
        "assets": entries,
    });
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&document)
            .map_err(|error| format!("serialize Honor asset audit failed: {error}"))?,
    )
    .map_err(|error| format!("write {} failed: {error}", output.display()))
}

fn print_build_report(
    reused: bool,
    source_identity: &str,
    manifest: &Path,
    catalog: &MappedRenderObjectStore,
    coverage: &HonorCoverage,
    missing_asset_keys: &[String],
    started: Instant,
) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": HONOR_BUILD_SCHEMA,
            "reused": reused,
            "source_identity": source_identity,
            "manifest": manifest,
            "manifest_sha256": catalog.manifest_sha256(),
            "coverage": coverage,
            "object_count": catalog.manifest().objects.len(),
            "page_count": catalog.manifest().pages.len(),
            "missing_asset_count": missing_asset_keys.len(),
            "missing_asset_keys": missing_asset_keys,
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        }))
        .map_err(|error| format!("serialize Honor build report failed: {error}"))?
    );
    Ok(())
}

impl HonorPlan {
    fn load(masterdata_dir: &Path) -> Result<Self, String> {
        let honors = load_json::<Vec<HonorEntry>>(masterdata_dir, "honors.json")?;
        let honor_groups = load_json::<Vec<HonorGroupEntry>>(masterdata_dir, "honorGroups.json")?;
        let bonds = load_json::<Vec<BondsPlanEntry>>(masterdata_dir, "bondsHonors.json")?;
        let words = load_json::<Vec<BondsHonorWordEntry>>(masterdata_dir, "bondsHonorWords.json")?;
        let units = load_json::<Vec<GameCharacterUnit>>(masterdata_dir, "gameCharacterUnits.json")?;
        let masterdata_identity = masterdata_identity(masterdata_dir)?;
        Self::from_tables(
            masterdata_identity,
            honors,
            honor_groups,
            bonds,
            words,
            units,
        )
    }

    fn from_tables(
        masterdata_identity: String,
        honors: Vec<HonorEntry>,
        honor_groups: Vec<HonorGroupEntry>,
        bonds: Vec<BondsPlanEntry>,
        words: Vec<BondsHonorWordEntry>,
        units: Vec<GameCharacterUnit>,
    ) -> Result<Self, String> {
        let words_by_group =
            words
                .iter()
                .fold(BTreeMap::<i32, Vec<i64>>::new(), |mut output, word| {
                    output
                        .entry(word.bonds_group_id)
                        .or_default()
                        .push(i64::from(word.id));
                    output
                });
        let mut objects = Vec::new();
        let mut coverage = HonorCoverage::default();
        for honor in &honors {
            for level in nonempty_levels(honor.levels.iter().map(|value| value.level)) {
                for full_size in [false, true] {
                    let key = standard_honor_object_key(honor.id, level, full_size);
                    objects.push(PlannedHonorObject {
                        key,
                        recipe: HonorObjectRecipe::Standard {
                            honor_id: honor.id,
                            honor_level: level,
                            full_size,
                        },
                    });
                    add_coverage(&mut coverage, false, full_size)?;
                }
            }
        }
        for honor in &bonds {
            let word_ids = words_by_group.get(&honor.bonds_group_id).ok_or_else(|| {
                format!(
                    "BondsHonor {} group {} has no words",
                    honor.id, honor.bonds_group_id
                )
            })?;
            let vs_values: &[bool] = if honor.configurable_unit_virtual_singer {
                &[false, true]
            } else {
                &[false]
            };
            for level in nonempty_levels(honor.levels.iter().map(|value| value.level)) {
                for inverse in [false, true] {
                    for &use_unit_virtual_singer in vs_values {
                        objects.push(PlannedHonorObject {
                            key: bonds_honor_object_key(
                                honor.id,
                                level,
                                false,
                                0,
                                inverse,
                                use_unit_virtual_singer,
                            ),
                            recipe: HonorObjectRecipe::Bonds {
                                honor_id: honor.id,
                                honor_level: level,
                                full_size: false,
                                word_id: 0,
                                inverse,
                                use_unit_virtual_singer,
                            },
                        });
                        add_coverage(&mut coverage, true, false)?;
                        for &word_id in word_ids {
                            objects.push(PlannedHonorObject {
                                key: bonds_honor_object_key(
                                    honor.id,
                                    level,
                                    true,
                                    word_id,
                                    inverse,
                                    use_unit_virtual_singer,
                                ),
                                recipe: HonorObjectRecipe::Bonds {
                                    honor_id: honor.id,
                                    honor_level: level,
                                    full_size: true,
                                    word_id,
                                    inverse,
                                    use_unit_virtual_singer,
                                },
                            });
                            add_coverage(&mut coverage, true, true)?;
                        }
                    }
                }
            }
        }
        objects.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        if objects.windows(2).any(|pair| pair[0].key == pair[1].key) {
            return Err("Honor plan contains duplicate canonical object keys".into());
        }
        coverage.total =
            u64::try_from(objects.len()).map_err(|_| "Honor object count overflow".to_string())?;
        let provider = Arc::new(HonorMasterDataProvider {
            honors: honors.into_iter().map(|value| (value.id, value)).collect(),
            honor_groups: honor_groups
                .into_iter()
                .map(|value| (value.id, value))
                .collect(),
            bonds: bonds.into_iter().map(|value| (value.id, value)).collect(),
            words: words
                .into_iter()
                .map(|value| (i64::from(value.id), value))
                .collect(),
            units,
        });
        Ok(Self {
            masterdata_identity,
            provider,
            objects,
            coverage,
        })
    }
}

fn add_coverage(coverage: &mut HonorCoverage, bonds: bool, full_size: bool) -> Result<(), String> {
    let count = match (bonds, full_size) {
        (false, false) => &mut coverage.standard_sub,
        (false, true) => &mut coverage.standard_main,
        (true, false) => &mut coverage.bonds_sub,
        (true, true) => &mut coverage.bonds_main,
    };
    *count = count.saturating_add(1);
    let width = if full_size { 380u64 } else { 180u64 };
    coverage.payload_bytes = coverage
        .payload_bytes
        .checked_add(width * 80 * 4)
        .ok_or_else(|| "Honor payload byte count overflow".to_string())?;
    Ok(())
}

fn nonempty_levels(levels: impl Iterator<Item = i32>) -> Vec<i32> {
    let levels = levels.collect::<Vec<_>>();
    if levels.is_empty() {
        vec![1]
    } else {
        levels
    }
}

struct RenderedObject {
    width: u32,
    height: u32,
    row_bytes: u32,
    pixels: Vec<u8>,
}

fn render_object(
    object: &PlannedHonorObject,
    md: &MasterData,
    assets: &AssetStore,
) -> Result<RenderedObject, String> {
    let full_size = match object.recipe {
        HonorObjectRecipe::Standard { full_size, .. }
        | HonorObjectRecipe::Bonds { full_size, .. } => full_size,
    };
    let width = if full_size { 380u32 } else { 180u32 };
    let height = 80u32;
    let row_bytes = width * 4;
    let mut surface = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
        .ok_or_else(|| format!("create Honor surface failed for {}", object.key))?;
    surface.canvas().clear(Color::TRANSPARENT);
    surface
        .canvas()
        .translate((width as f32 / 2.0, height as f32 / 2.0));
    match object.recipe {
        HonorObjectRecipe::Standard {
            honor_id,
            honor_level,
            full_size,
        } => allium_renderer::elements::honor::render_static_honor(
            surface.canvas(),
            honor_id,
            honor_level,
            full_size,
            md,
            assets,
        ),
        HonorObjectRecipe::Bonds {
            honor_id,
            honor_level,
            full_size,
            word_id,
            inverse,
            use_unit_virtual_singer,
        } => allium_renderer::elements::honor::render_bonds_honor(
            surface.canvas(),
            honor_id,
            honor_level,
            full_size,
            word_id,
            inverse,
            use_unit_virtual_singer,
            md,
            assets,
        ),
    }
    let mut pixels = vec![0u8; row_bytes as usize * height as usize];
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    if !surface.read_pixels(&info, &mut pixels, row_bytes as usize, (0, 0)) {
        return Err(format!("read Honor pixels failed for {}", object.key));
    }
    Ok(RenderedObject {
        width,
        height,
        row_bytes,
        pixels,
    })
}

fn enforce_expected_count(coverage: &HonorCoverage, expected: Option<u64>) -> Result<(), String> {
    if expected.is_some_and(|value| value != coverage.total) {
        return Err(format!(
            "Honor coverage gate failed: expected {}, planned {}",
            expected.unwrap_or_default(),
            coverage.total
        ));
    }
    Ok(())
}

fn enforce_manifest_coverage(
    catalog: &MappedRenderObjectStore,
    coverage: &HonorCoverage,
) -> Result<(), String> {
    let standard = catalog
        .manifest()
        .objects
        .iter()
        .filter(|entry| entry.kind == RenderObjectKind::StandardHonor)
        .count() as u64;
    let bonds = catalog
        .manifest()
        .objects
        .iter()
        .filter(|entry| entry.kind == RenderObjectKind::BondsHonor)
        .count() as u64;
    let expected_standard = coverage.standard_main + coverage.standard_sub;
    let expected_bonds = coverage.bonds_main + coverage.bonds_sub;
    if catalog.manifest().objects.len() as u64 != coverage.total
        || standard != expected_standard
        || bonds != expected_bonds
    {
        return Err(format!(
            "published Honor coverage mismatch: total {}/{}, standard {}/{}, bonds {}/{}",
            catalog.manifest().objects.len(),
            coverage.total,
            standard,
            expected_standard,
            bonds,
            expected_bonds
        ));
    }
    Ok(())
}

fn parse_expected_count(value: Option<&String>) -> Result<Option<u64>, String> {
    value
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|error| format!("invalid expected object count: {error}"))
        })
        .transpose()
}

fn masterdata_identity(root: &Path) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"allium.honor-masterdata-input.v1");
    for name in [
        "honors.json",
        "honorGroups.json",
        "bondsHonors.json",
        "bondsHonorWords.json",
        "gameCharacterUnits.json",
    ] {
        let bytes = std::fs::read(root.join(name))
            .map_err(|error| format!("read {} failed: {error}", root.join(name).display()))?;
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update(Sha256::digest(&bytes));
    }
    Ok(hex::encode(digest.finalize()))
}

fn honor_source_identity(masterdata_identity: &str, asset_identity: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(HONOR_RENDER_OBJECT_CONTRACT.as_bytes());
    digest.update(masterdata_identity.as_bytes());
    digest.update(asset_identity.as_bytes());
    hex::encode(digest.finalize())
}

fn object_source_identity(source_identity: &str, key: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(source_identity.as_bytes());
    digest.update((key.len() as u64).to_le_bytes());
    digest.update(key.as_bytes());
    hex::encode(digest.finalize())
}

fn load_json<T: serde::de::DeserializeOwned>(root: &Path, name: &str) -> Result<T, String> {
    let path = root.join(name);
    serde_json::from_slice(
        &std::fs::read(&path)
            .map_err(|error| format!("read {} failed: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {} failed: {error}", path.display()))
}

impl MasterDataProvider for HonorMasterDataProvider {
    fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
        None
    }

    fn get_card(&self, _: i32) -> Option<CardEntry> {
        None
    }

    fn resolve_color(&self, _: i32) -> Option<ResolvedColor> {
        None
    }

    fn resolve_font(&self, _: i32) -> Option<String> {
        None
    }

    fn resolve_stamp(&self, _: i32) -> Option<String> {
        None
    }

    fn resolve_resource(&self, _: &str, _: i32) -> Option<ResourceInfo> {
        None
    }

    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor> {
        let honor = self.honors.get(&honor_id)?;
        let is_live_master = honor.honor_mission_type.is_some() && honor.assetbundle_name.is_none();
        let (asset_bundle_name, honor_rarity) = if is_live_master {
            let level = honor.levels.iter().find(|value| value.level == honor_level);
            (
                level
                    .and_then(|value| value.assetbundle_name.clone())
                    .unwrap_or_default(),
                level
                    .and_then(|value| value.honor_rarity.clone())
                    .unwrap_or_else(|| "low".into()),
            )
        } else {
            (
                honor.assetbundle_name.clone().unwrap_or_default(),
                honor.honor_rarity.clone().unwrap_or_else(|| "low".into()),
            )
        };
        let group = honor
            .group_id
            .filter(|id| *id > 0)
            .and_then(|id| self.honor_groups.get(&id));
        Some(ResolvedHonor {
            asset_bundle_name,
            honor_rarity,
            honor_type: group
                .map(|value| value.honor_type.clone())
                .unwrap_or_else(|| "normal".into()),
            background_asset_bundle_name: group
                .and_then(|value| value.background_assetbundle_name.clone())
                .filter(|value| !value.is_empty()),
            frame_name: group
                .and_then(|value| value.frame_name.clone())
                .filter(|value| !value.is_empty()),
            is_live_master,
            has_star: honor.levels.len() > 1,
            honor_level,
            honor_mission_type: honor.honor_mission_type.clone(),
        })
    }

    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        let value = self.bonds.get(&id)?;
        Some(BondsHonorEntry {
            id: value.id,
            game_character_unit_id1: value.game_character_unit_id1,
            game_character_unit_id2: value.game_character_unit_id2,
            honor_rarity: value.honor_rarity.clone(),
            configurable_unit_virtual_singer: value.configurable_unit_virtual_singer,
        })
    }

    fn get_bonds_honor_word(&self, word_id: i64) -> Option<BondsHonorWordEntry> {
        self.words.get(&word_id).cloned()
    }

    fn get_honor(&self, honor_id: i32) -> Option<HonorEntry> {
        self.honors.get(&honor_id).cloned()
    }

    fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
        let Some(self_unit) = self.units.iter().find(|value| value.id == self_id) else {
            return self_id;
        };
        if self_unit.game_character_id < 21 {
            return self_id;
        }
        let Some(partner) = self.units.iter().find(|value| value.id == partner_id) else {
            return self_id;
        };
        self.units
            .iter()
            .find(|value| {
                value.game_character_id == self_unit.game_character_id && value.unit == partner.unit
            })
            .map(|value| value.id)
            .unwrap_or(self_id)
    }

    fn font_count(&self) -> usize {
        0
    }

    fn color_count(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use allium_renderer::types::HonorLevelEntry;

    fn level(level: i32) -> HonorLevelEntry {
        HonorLevelEntry {
            level,
            assetbundle_name: None,
            honor_rarity: None,
            description: None,
        }
    }

    #[test]
    fn plan_canonicalizes_bonds_sub_and_counts_each_axis_once() {
        let plan = HonorPlan::from_tables(
            "fixture".into(),
            vec![HonorEntry {
                id: 1,
                assetbundle_name: Some("honor".into()),
                honor_rarity: Some("low".into()),
                group_id: None,
                levels: vec![level(1), level(2)],
                honor_mission_type: None,
            }],
            Vec::new(),
            vec![BondsPlanEntry {
                id: 2,
                bonds_group_id: 7,
                game_character_unit_id1: 1,
                game_character_unit_id2: 2,
                honor_rarity: "low".into(),
                configurable_unit_virtual_singer: true,
                levels: vec![PlanLevel { level: 1 }],
            }],
            vec![
                BondsHonorWordEntry {
                    id: 11,
                    assetbundle_name: "a".into(),
                    bonds_group_id: 7,
                    seq: 1,
                },
                BondsHonorWordEntry {
                    id: 12,
                    assetbundle_name: "b".into(),
                    bonds_group_id: 7,
                    seq: 2,
                },
            ],
            Vec::new(),
        )
        .expect("plan");
        assert_eq!(
            plan.coverage,
            HonorCoverage {
                standard_main: 2,
                standard_sub: 2,
                bonds_main: 8,
                bonds_sub: 4,
                total: 16,
                payload_bytes: 1_561_600,
            }
        );
        let bonds_sub = plan
            .objects
            .iter()
            .filter(|object| object.key.starts_with("bonds_honor:") && object.key.ends_with("/sub"))
            .collect::<Vec<_>>();
        assert_eq!(bonds_sub.len(), 4);
        assert!(bonds_sub
            .iter()
            .all(|object| object.key.contains("/word-00000000/")));
    }

    #[test]
    fn audit_only_blocks_assets_declared_by_the_snapshot() {
        let missing = vec![
            "honor/declared/degree_main".to_string(),
            "honor/masterdata_only/degree_main".to_string(),
        ];
        let available = BTreeSet::from(["honor/declared/degree_main".to_string()]);
        assert_eq!(
            required_missing_assets(&missing, Some(&available)),
            vec!["honor/declared/degree_main".to_string()]
        );
        assert_eq!(required_missing_assets(&missing, None), missing);
    }
}
