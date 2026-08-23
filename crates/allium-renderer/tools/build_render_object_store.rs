use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use allium_renderer::compiled_profile::{CompiledProfileBatch, CompiledResourceRequest};
use allium_renderer::render_object::{
    MappedRenderObjectStore, RenderObjectKind, RenderObjectManifest, RenderObjectStoreWriter,
    RenderObjectWrite,
};
use allium_renderer::render_object_catalog::{
    DesiredRenderObject, DesiredRenderObjectCatalog, RenderObjectDependency,
};
mod render_object_honor;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use skia_safe::{AlphaType, ColorType, Data, Image, ImageInfo};

const BUILD_LIST_SCHEMA: &str = "allium.render-object-build-list.v1";
const DEFAULT_PAGE_MIB: u64 = 512;

#[derive(Debug, Deserialize)]
struct BuildList {
    schema: String,
    source_identity: String,
    objects: Vec<BuildObject>,
}

#[derive(Debug, Deserialize)]
struct BuildObject {
    key: String,
    kind: RenderObjectKind,
    source_path: String,
    #[serde(default)]
    source_sha256: Option<String>,
    #[serde(default)]
    source_identity: Option<String>,
}

struct DecodedObject {
    source_sha256: String,
    width: u32,
    height: u32,
    row_bytes: u32,
    pixels: Vec<u8>,
}

#[derive(Debug)]
struct GlobalObjectSource {
    key: String,
    source_path: PathBuf,
    source_sha256: String,
    width: u32,
    height: u32,
    pixel_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct CompiledBatchReport {
    batch: CompiledProfileBatch,
}

#[derive(Debug, Deserialize)]
struct PlannedAssetFile {
    logical_path: String,
    object_key: String,
    sha256: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("--honor-plan") {
        return render_object_honor::run_honor_plan(&args);
    }
    if args.get(1).map(String::as_str) == Some("--honor-store") {
        return render_object_honor::run_honor_store(&args);
    }
    if args.get(1).map(String::as_str) == Some("--honor-delta-store") {
        return render_object_honor::run_honor_delta_store(&args);
    }
    if args.get(1).map(String::as_str) == Some("--honor-assets-audit") {
        return render_object_honor::run_honor_assets_audit(&args);
    }
    if args.get(1).map(String::as_str) == Some("--global-cache-plan") {
        return run_global_cache(&args, false);
    }
    if args.get(1).map(String::as_str) == Some("--global-cache") {
        return run_global_cache(&args, true);
    }
    if args.get(1).map(String::as_str) == Some("--compiled-report") {
        return run_compiled_report(&args);
    }
    if args.get(1).map(String::as_str) == Some("--merge-stores") {
        return run_merge_stores(&args);
    }
    if args.get(1).map(String::as_str) == Some("--catalog-diff") {
        return run_catalog_diff(&args);
    }
    if args.get(1).map(String::as_str) == Some("--catalog-from-stores") {
        return run_catalog_from_stores(&args);
    }
    if args.get(1).map(String::as_str) == Some("--plan-desired-catalog") {
        return run_plan_desired_catalog(&args);
    }
    if args.get(1).map(String::as_str) == Some("--deck-art-store") {
        return run_deck_art_store(&args);
    }
    if args.get(1).map(String::as_str) == Some("--filter-store") {
        return run_filter_store(&args);
    }
    if !(3..=4).contains(&args.len()) {
        return Err(format!(
            "usage: {} <build-list.json> <output-dir> [page-mib]",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let build_list_path = PathBuf::from(&args[1]);
    let output_dir = PathBuf::from(&args[2]);
    let page_mib = args
        .get(3)
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|error| format!("invalid page-mib: {error}"))?
        .unwrap_or(DEFAULT_PAGE_MIB);
    let page_payload_limit = page_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| "page-mib overflow".to_string())?;

    let started = Instant::now();
    let bytes = std::fs::read(&build_list_path)
        .map_err(|error| format!("read {} failed: {error}", build_list_path.display()))?;
    let mut list: BuildList = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {} failed: {error}", build_list_path.display()))?;
    if list.schema != BUILD_LIST_SCHEMA || list.source_identity.trim().is_empty() {
        return Err(format!(
            "unsupported build-list schema or empty source identity: {}",
            list.schema
        ));
    }
    list.objects
        .sort_unstable_by(|left, right| left.key.cmp(&right.key));
    let source_root = build_list_path.parent().unwrap_or_else(|| Path::new("."));
    let mut writer =
        RenderObjectStoreWriter::create(&output_dir, list.source_identity, page_payload_limit)
            .map_err(|error| error.to_string())?;
    let mut decoded_bytes = 0u64;
    for object in &list.objects {
        let source_path = resolve_source_path(source_root, &object.source_path)?;
        let decoded = decode_premul_rgba8(&source_path, object.source_sha256.as_deref())?;
        decoded_bytes = decoded_bytes.saturating_add(decoded.pixels.len() as u64);
        writer
            .add(RenderObjectWrite {
                key: &object.key,
                kind: object.kind,
                source_sha256: object
                    .source_identity
                    .as_deref()
                    .unwrap_or(&decoded.source_sha256),
                width: decoded.width,
                height: decoded.height,
                row_bytes: decoded.row_bytes,
                pixels: &decoded.pixels,
            })
            .map_err(|error| error.to_string())?;
    }
    let manifest_path = writer.finish().map_err(|error| error.to_string())?;
    let store = MappedRenderObjectStore::open(&manifest_path).map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "allium.render-object-build-report.v1",
            "build_list": build_list_path,
            "manifest": manifest_path,
            "manifest_sha256": store.manifest_sha256(),
            "source_identity": store.manifest().source_identity,
            "object_count": store.manifest().objects.len(),
            "page_count": store.manifest().pages.len(),
            "decoded_bytes": decoded_bytes,
            "mapped_bytes": store.mapped_bytes(),
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        }))
        .map_err(|error| format!("serialize build report failed: {error}"))?
    );
    Ok(())
}

fn run_catalog_from_stores(args: &[String]) -> Result<(), String> {
    if args.len() < 5 {
        return Err(format!(
            "usage: {} --catalog-from-stores <catalog-header.json> <output-catalog.json> <manifest.json>...",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let header_path = PathBuf::from(&args[2]);
    let output_path = PathBuf::from(&args[3]);
    let mut catalog: DesiredRenderObjectCatalog = serde_json::from_slice(
        &std::fs::read(&header_path)
            .map_err(|error| format!("read {} failed: {error}", header_path.display()))?,
    )
    .map_err(|error| format!("parse {} failed: {error}", header_path.display()))?;
    catalog.objects.clear();
    for manifest_arg in &args[4..] {
        let manifest_path = PathBuf::from(manifest_arg);
        let store = MappedRenderObjectStore::open_metadata_catalog(&manifest_path)
            .map_err(|error| format!("open {} failed: {error}", manifest_path.display()))?;
        for entry in &store.manifest().objects {
            let kind = match entry.kind {
                RenderObjectKind::Texture => "texture",
                RenderObjectKind::StandardHonor => "standard_honor",
                RenderObjectKind::BondsHonor => "bonds_honor",
                RenderObjectKind::CardMember | RenderObjectKind::Component => "component",
            };
            let recipe_contract = match entry.kind {
                RenderObjectKind::Texture => "allium.render-object.texture.decode-premul-rgba8.v1",
                RenderObjectKind::StandardHonor | RenderObjectKind::BondsHonor => {
                    allium_renderer::render_object::HONOR_RENDER_OBJECT_CONTRACT
                }
                RenderObjectKind::CardMember => {
                    allium_renderer::profile_compositor::DECK_ART_VARIANT_CONTRACT
                }
                RenderObjectKind::Component => {
                    allium_renderer::profile_compositor::GENERAL_BASE_CONTRACT
                }
            };
            let dependencies = if let Some(asset_path) = entry.key.strip_prefix("texture:assets/") {
                let logical_path = format!("{asset_path}.png");
                let object_key = format!(
                    "asset-blobs/sha256/{}/{}",
                    &entry.source_sha256[..2],
                    entry.source_sha256
                );
                vec![RenderObjectDependency {
                    kind: "asset_blob".into(),
                    key: logical_path.clone(),
                    logical_path,
                    object_key,
                    sha256: entry.source_sha256.clone(),
                }]
            } else {
                vec![RenderObjectDependency {
                    kind: "builder_static".into(),
                    key: entry.key.clone(),
                    logical_path: String::new(),
                    object_key: manifest_path.to_string_lossy().into_owned(),
                    sha256: entry.source_sha256.clone(),
                }]
            };
            catalog.objects.push(DesiredRenderObject {
                key: entry.key.clone(),
                kind: kind.into(),
                recipe_contract: recipe_contract.into(),
                recipe_sha256: entry.source_sha256.clone(),
                source_identity: String::new(),
                dependencies,
            });
        }
    }
    let sealed = catalog.seal()?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {} failed: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(&sealed)
        .map_err(|error| format!("serialize desired catalog failed: {error}"))?;
    let temporary = output_path.with_extension("json.tmp");
    std::fs::write(&temporary, &bytes)
        .map_err(|error| format!("write {} failed: {error}", temporary.display()))?;
    std::fs::rename(&temporary, &output_path)
        .map_err(|error| format!("publish {} failed: {error}", output_path.display()))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "allium.render-object-desired-catalog-build.v1",
            "catalog": output_path,
            "catalog_sha256": sealed.catalog_sha256,
            "object_count": sealed.objects.len(),
        }))
        .map_err(|error| format!("serialize catalog report failed: {error}"))?
    );
    Ok(())
}

fn run_plan_desired_catalog(args: &[String]) -> Result<(), String> {
    if args.len() != 7 {
        return Err(format!(
            "usage: {} --plan-desired-catalog <catalog-header.json> <masterdata-dir> <asset-files.json> <current-manifest.json> <output-catalog.json>",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let header_path = PathBuf::from(&args[2]);
    let masterdata_dir = PathBuf::from(&args[3]);
    let assets_path = PathBuf::from(&args[4]);
    let current_manifest_path = PathBuf::from(&args[5]);
    let output_path = PathBuf::from(&args[6]);
    let mut catalog: DesiredRenderObjectCatalog = serde_json::from_slice(
        &std::fs::read(&header_path)
            .map_err(|error| format!("read {} failed: {error}", header_path.display()))?,
    )
    .map_err(|error| format!("parse {} failed: {error}", header_path.display()))?;
    catalog.objects.clear();
    catalog
        .build_dependencies
        .retain(|dependency| dependency.kind == "masterdata");
    catalog.catalog_sha256.clear();

    let current = MappedRenderObjectStore::open_metadata_catalog(&current_manifest_path)
        .map_err(|error| format!("open current render-object catalog failed: {error}"))?;
    let current_by_key = current
        .manifest()
        .objects
        .iter()
        .map(|entry| (entry.key.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    for entry in &current.manifest().objects {
        if entry.key.starts_with("texture:assets/")
            || matches!(
                entry.kind,
                RenderObjectKind::StandardHonor | RenderObjectKind::BondsHonor
            )
            || entry.key.starts_with("component:deck-art-variant/")
        {
            continue;
        }
        let kind = match entry.kind {
            RenderObjectKind::Texture => "texture",
            RenderObjectKind::StandardHonor => "standard_honor",
            RenderObjectKind::BondsHonor => "bonds_honor",
            RenderObjectKind::CardMember | RenderObjectKind::Component => "component",
        };
        catalog.objects.push(DesiredRenderObject {
            key: entry.key.clone(),
            kind: kind.into(),
            recipe_contract: "allium.render-object.prebuilt.v1".into(),
            recipe_sha256: entry.source_sha256.clone(),
            source_identity: entry.source_sha256.clone(),
            dependencies: vec![RenderObjectDependency {
                kind: "builder_static".into(),
                key: entry.key.clone(),
                logical_path: String::new(),
                object_key: current_manifest_path.to_string_lossy().into_owned(),
                sha256: entry.source_sha256.clone(),
            }],
        });
    }

    let mut assets: Vec<PlannedAssetFile> = serde_json::from_slice(
        &std::fs::read(&assets_path)
            .map_err(|error| format!("read {} failed: {error}", assets_path.display()))?,
    )
    .map_err(|error| format!("parse {} failed: {error}", assets_path.display()))?;
    assets.retain(|asset| is_profile_asset_path(&asset.logical_path));
    assets.sort_unstable_by(|left, right| left.logical_path.cmp(&right.logical_path));
    assets.dedup_by(|left, right| left.logical_path == right.logical_path);
    let texture_contract = "allium.render-object.texture.decode-premul-rgba8.v2";
    let texture_recipe_sha = contract_sha256(texture_contract);
    for asset in &assets {
        if !is_sha256_string(&asset.sha256) || asset.object_key.trim().is_empty() {
            return Err(format!(
                "invalid planned asset identity for {}",
                asset.logical_path
            ));
        }
        let Some(key) = render_texture_key(&asset.logical_path) else {
            continue;
        };
        let dependency = asset_dependency(asset);
        let prebuilt = current_by_key
            .get(key.as_str())
            .is_some_and(|entry| entry.source_sha256 == asset.sha256);
        catalog.objects.push(DesiredRenderObject {
            key,
            kind: "texture".into(),
            recipe_contract: if prebuilt {
                "allium.render-object.prebuilt.v1".into()
            } else {
                texture_contract.into()
            },
            recipe_sha256: if prebuilt {
                asset.sha256.clone()
            } else {
                texture_recipe_sha.clone()
            },
            source_identity: if prebuilt {
                asset.sha256.clone()
            } else {
                String::new()
            },
            dependencies: vec![dependency],
        });
        if is_honor_asset_path(&asset.logical_path) {
            catalog.build_dependencies.push(asset_dependency(asset));
        }
    }

    let texture_sealed = catalog.clone().seal()?;
    let deck_contract = allium_renderer::profile_compositor::DECK_ART_VARIANT_CONTRACT;
    let deck_recipe_sha = contract_sha256(deck_contract);
    let mut deck_variant_sources = BTreeMap::<String, (String, String)>::new();
    for texture in texture_sealed.objects.iter().filter(|object| {
        object
            .key
            .starts_with("texture:assets/character/member_cutout/")
    }) {
        let (key, _) = allium_renderer::profile_compositor::deck_art_variant_identity_from_source(
            &texture.source_identity,
        );
        let dependency = texture
            .dependencies
            .first()
            .ok_or_else(|| format!("deck-art source {} has no dependency", texture.key))?;
        let blob_identity = (dependency.sha256.clone(), dependency.object_key.clone());
        if let Some(previous) = deck_variant_sources.get(&key) {
            if previous != &blob_identity {
                return Err(format!(
                    "deck-art variant {key} has conflicting blob identities"
                ));
            }
            continue;
        }
        deck_variant_sources.insert(key.clone(), blob_identity);
        catalog.objects.push(DesiredRenderObject {
            key,
            kind: "component".into(),
            recipe_contract: deck_contract.into(),
            recipe_sha256: deck_recipe_sha.clone(),
            source_identity: String::new(),
            dependencies: texture.dependencies.clone(),
        });
    }

    let mut honor_asset_digest = Sha256::new();
    for dependency in &catalog.build_dependencies {
        honor_asset_digest.update(dependency.logical_path.as_bytes());
        honor_asset_digest.update(dependency.sha256.as_bytes());
    }
    let honor_asset_identity = hex::encode(honor_asset_digest.finalize());
    let honor_contract = allium_renderer::render_object::HONOR_RENDER_OBJECT_CONTRACT;
    let honor_recipe_sha = contract_sha256(honor_contract);
    let honor_dependencies = vec![
        RenderObjectDependency {
            kind: "masterdata".into(),
            key: catalog.masterdata_object_key.clone(),
            logical_path: String::new(),
            object_key: catalog.masterdata_object_key.clone(),
            sha256: catalog.masterdata_sha256.clone(),
        },
        RenderObjectDependency {
            kind: "builder_static".into(),
            key: "honor-asset-set".into(),
            logical_path: String::new(),
            object_key: String::new(),
            sha256: honor_asset_identity,
        },
    ];
    for (key, kind) in render_object_honor::planned_honor_catalog_keys(&masterdata_dir)? {
        catalog.objects.push(DesiredRenderObject {
            key,
            kind: kind.into(),
            recipe_contract: honor_contract.into(),
            recipe_sha256: honor_recipe_sha.clone(),
            source_identity: String::new(),
            dependencies: honor_dependencies.clone(),
        });
    }

    let sealed = catalog.seal()?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {} failed: {error}", parent.display()))?;
    }
    let temporary = output_path.with_extension("json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&sealed)
            .map_err(|error| format!("serialize desired catalog failed: {error}"))?,
    )
    .map_err(|error| format!("write {} failed: {error}", temporary.display()))?;
    std::fs::rename(&temporary, &output_path)
        .map_err(|error| format!("publish {} failed: {error}", output_path.display()))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "allium.render-object-desired-catalog-build.v1",
            "catalog": output_path,
            "catalog_sha256": sealed.catalog_sha256,
            "object_count": sealed.objects.len(),
            "build_dependency_count": sealed.build_dependencies.len(),
        }))
        .map_err(|error| format!("serialize catalog report failed: {error}"))?
    );
    Ok(())
}

fn asset_dependency(asset: &PlannedAssetFile) -> RenderObjectDependency {
    RenderObjectDependency {
        kind: "asset_blob".into(),
        key: format!("asset:{}", asset.logical_path),
        logical_path: asset.logical_path.clone(),
        object_key: asset.object_key.clone(),
        sha256: asset.sha256.clone(),
    }
}

fn render_texture_key(logical_path: &str) -> Option<String> {
    let normalized = logical_path.trim_start_matches('/').replace('\\', "/");
    let (base, extension) = normalized.rsplit_once('.')?;
    if !matches!(
        extension.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg"
    ) {
        return None;
    }
    Some(format!("texture:assets/{base}"))
}

fn is_profile_asset_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase().replace('\\', "/");
    path.contains("character/member_small/")
        || path.contains("character/member_cutout/")
        || path.contains("thumbnail/chara/")
        || path.contains("chara_avatar/")
        || path.contains("custom_profile")
        || path.starts_with("stamp/")
        || path.contains("/stamp/")
        || ((path.contains("event_story") || path.contains("unit_story"))
            && path.contains("banner"))
        || is_honor_asset_path(&path)
}

fn is_honor_asset_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase().replace('\\', "/");
    path.contains("honor/")
        || path.contains("bonds_honor/")
        || path.contains("rank_live/")
        || path.contains("degree/")
}

fn contract_sha256(contract: &str) -> String {
    hex::encode(Sha256::digest(contract.as_bytes()))
}

fn is_sha256_string(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}

fn run_catalog_diff(args: &[String]) -> Result<(), String> {
    if args.len() != 4 {
        return Err(format!(
            "usage: {} --catalog-diff <desired-catalog.json> <current-manifest.json>",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let catalog_path = PathBuf::from(&args[2]);
    let current_manifest_path = PathBuf::from(&args[3]);
    let catalog: DesiredRenderObjectCatalog = serde_json::from_slice(
        &std::fs::read(&catalog_path)
            .map_err(|error| format!("read {} failed: {error}", catalog_path.display()))?,
    )
    .map_err(|error| format!("parse {} failed: {error}", catalog_path.display()))?;
    let sealed = catalog.seal()?;
    let current = MappedRenderObjectStore::open_metadata_catalog(&current_manifest_path)
        .map_err(|error| error.to_string())?;
    let diff = sealed.diff_against(current.manifest())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "allium.render-object-catalog-diff.v1",
            "catalog_sha256": sealed.catalog_sha256,
            "desired_object_count": sealed.objects.len(),
            "current_manifest_sha256": current.manifest_sha256(),
            "reuse_count": diff.reuse.len(),
            "build_count": diff.build.len(),
            "remove_count": diff.remove.len(),
            "diff": diff,
        }))
        .map_err(|error| format!("serialize catalog diff failed: {error}"))?
    );
    Ok(())
}

fn run_merge_stores(args: &[String]) -> Result<(), String> {
    if args.len() != 5 {
        return Err(format!(
            "usage: {} --merge-stores <output-root> <base-manifest> <delta-manifest>",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let started = Instant::now();
    let output_root = PathBuf::from(&args[2]);
    let base_manifest_path = PathBuf::from(&args[3]);
    let delta_manifest_path = PathBuf::from(&args[4]);
    let base_store = MappedRenderObjectStore::open_metadata_catalog(&base_manifest_path)
        .map_err(|error| format!("open base store failed: {error}"))?;
    let delta_store = MappedRenderObjectStore::open_metadata_catalog(&delta_manifest_path)
        .map_err(|error| format!("open delta store failed: {error}"))?;
    let mut identity = Sha256::new();
    identity.update(b"allium.merged-render-object-store.profile-warm-v2");
    identity.update(base_store.manifest_sha256().as_bytes());
    identity.update(delta_store.manifest_sha256().as_bytes());
    let source_identity = hex::encode(identity.finalize());
    let output_dir = output_root.join(&source_identity);
    let manifest_path = output_dir.join("manifest.json");
    if manifest_path.is_file() {
        let store = MappedRenderObjectStore::open_metadata_catalog(&manifest_path)
            .map_err(|error| format!("open reused merged store failed: {error}"))?;
        print_merge_report(true, &source_identity, &manifest_path, &store, started)?;
        return Ok(());
    }
    if output_dir.exists() {
        return Err(format!(
            "merged store output exists without a valid manifest: {}",
            output_dir.display()
        ));
    }
    std::fs::create_dir_all(&output_dir)
        .map_err(|error| format!("create {} failed: {error}", output_dir.display()))?;
    let merged = merge_store_manifests(
        &output_dir,
        &base_manifest_path,
        base_store.manifest(),
        &delta_manifest_path,
        delta_store.manifest(),
        format!("global-merged:{source_identity}"),
    )?;
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&merged)
            .map_err(|error| format!("serialize merged manifest failed: {error}"))?,
    )
    .map_err(|error| format!("write {} failed: {error}", manifest_path.display()))?;
    let store = MappedRenderObjectStore::open_metadata_catalog(&manifest_path)
        .map_err(|error| format!("verify merged store failed: {error}"))?;
    print_merge_report(false, &source_identity, &manifest_path, &store, started)
}

fn merge_store_manifests(
    output_dir: &Path,
    base_manifest_path: &Path,
    base: &RenderObjectManifest,
    delta_manifest_path: &Path,
    delta: &RenderObjectManifest,
    source_identity: String,
) -> Result<RenderObjectManifest, String> {
    if (
        base.schema.as_str(),
        base.generator_contract.as_str(),
        base.pixel_format.as_str(),
    ) != (
        delta.schema.as_str(),
        delta.generator_contract.as_str(),
        delta.pixel_format.as_str(),
    ) {
        return Err("base and delta render-object contracts differ".into());
    }
    let base_page_count = u16::try_from(base.pages.len())
        .map_err(|_| "base render-object store has too many pages".to_string())?;
    let total_page_count = base
        .pages
        .len()
        .checked_add(delta.pages.len())
        .ok_or_else(|| "merged page count overflow".to_string())?;
    if total_page_count > usize::from(u16::MAX) + 1 {
        return Err("merged render-object store has too many pages".into());
    }
    let mut pages = Vec::with_capacity(total_page_count);
    for (manifest_path, manifest) in [(base_manifest_path, base), (delta_manifest_path, delta)] {
        let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        for page in &manifest.pages {
            let source = root.join(&page.file);
            let file = format!("page-{:04}.rgba", pages.len());
            let destination = output_dir.join(&file);
            std::fs::hard_link(&source, &destination).map_err(|error| {
                format!(
                    "hard-link {} to {} failed: {error}",
                    source.display(),
                    destination.display()
                )
            })?;
            let mut page = page.clone();
            page.file = file;
            pages.push(page);
        }
    }
    let replacement_keys = delta
        .objects
        .iter()
        .map(|object| object.key.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut objects = base
        .objects
        .iter()
        .filter(|object| !replacement_keys.contains(object.key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    for object in &delta.objects {
        let mut object = object.clone();
        object.page = object
            .page
            .checked_add(base_page_count)
            .ok_or_else(|| format!("page index overflow for {}", object.key))?;
        objects.push(object);
    }
    // Pixel verification follows manifest order and therefore doubles as the
    // worker's deterministic page-cache warmup. Keep the compact card images
    // used by dense profile collages at the tail without baking in any player
    // or profile-specific key list.
    objects.sort_unstable_by(|left, right| {
        profile_warmth_class(&left.key)
            .cmp(&profile_warmth_class(&right.key))
            .then_with(|| left.key.cmp(&right.key))
    });
    Ok(RenderObjectManifest {
        schema: base.schema.clone(),
        generator_contract: base.generator_contract.clone(),
        pixel_format: base.pixel_format.clone(),
        source_identity,
        pages,
        objects,
    })
}

fn run_deck_art_store(args: &[String]) -> Result<(), String> {
    if !(4..=5).contains(&args.len()) {
        return Err(format!(
            "usage: {} --deck-art-store <output-root> <texture-manifest> [page-mib]",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let output_root = PathBuf::from(&args[2]);
    let source_manifest = PathBuf::from(&args[3]);
    let page_mib = args
        .get(4)
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|error| format!("invalid page-mib: {error}"))?
        .unwrap_or(DEFAULT_PAGE_MIB);
    let page_limit = page_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| "page-mib overflow".to_string())?;
    let source = MappedRenderObjectStore::open(&source_manifest)
        .map_err(|error| format!("open deck source store failed: {error}"))?;
    let mut planned = BTreeMap::<String, String>::new();
    for entry in &source.manifest().objects {
        if !entry
            .key
            .starts_with("texture:assets/character/member_cutout/")
        {
            continue;
        }
        let object = source
            .object(&entry.key)
            .ok_or_else(|| format!("missing mapped deck source {}", entry.key))?;
        let (key, _) = allium_renderer::profile_compositor::deck_art_variant_identity(object);
        planned.entry(key).or_insert_with(|| entry.key.clone());
    }
    if planned.is_empty() {
        return Err("render-object store has no member_cutout deck sources".into());
    }
    let mut identity = Sha256::new();
    identity.update(allium_renderer::profile_compositor::DECK_ART_VARIANT_CONTRACT.as_bytes());
    for (key, source_key) in &planned {
        identity.update((key.len() as u64).to_le_bytes());
        identity.update(key.as_bytes());
        identity.update(source_key.as_bytes());
    }
    let source_identity = hex::encode(identity.finalize());
    let output_dir = output_root.join(&source_identity);
    let manifest = output_dir.join("manifest.json");
    if manifest.is_file() {
        let store = MappedRenderObjectStore::open_metadata_catalog(&manifest)
            .map_err(|error| format!("open reused deck-art store failed: {error}"))?;
        return print_simple_store_report(
            "allium.deck-art-variant-build.v1",
            true,
            &manifest,
            &store,
        );
    }
    if output_dir.exists() {
        return Err(format!(
            "deck-art output exists without a valid manifest: {}",
            output_dir.display()
        ));
    }
    let mut writer = RenderObjectStoreWriter::create(
        &output_dir,
        format!("deck-art:{}", source_identity),
        page_limit,
    )
    .map_err(|error| error.to_string())?;
    for (expected_key, source_key) in planned {
        let object = source
            .object(&source_key)
            .ok_or_else(|| format!("missing mapped deck source {source_key}"))?;
        let built = allium_renderer::profile_compositor::build_deck_art_variant_simd(object)
            .map_err(|error| error.to_string())?;
        if built.object_key != expected_key {
            return Err(format!("deck-art identity changed for {source_key}"));
        }
        writer
            .add(RenderObjectWrite {
                key: &built.object_key,
                kind: RenderObjectKind::Component,
                source_sha256: &built.source_sha256,
                width: built.width,
                height: built.height,
                row_bytes: built.row_bytes,
                pixels: &built.pixels,
            })
            .map_err(|error| error.to_string())?;
    }
    let manifest = writer.finish().map_err(|error| error.to_string())?;
    let store = MappedRenderObjectStore::open_metadata_catalog(&manifest)
        .map_err(|error| format!("verify deck-art store failed: {error}"))?;
    print_simple_store_report("allium.deck-art-variant-build.v1", false, &manifest, &store)
}

fn run_filter_store(args: &[String]) -> Result<(), String> {
    if args.len() != 5 {
        return Err(format!(
            "usage: {} --filter-store <output-root> <source-manifest> <desired-keys.json>",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let output_root = PathBuf::from(&args[2]);
    let source_manifest = PathBuf::from(&args[3]);
    let desired = serde_json::from_slice::<Vec<String>>(
        &std::fs::read(&args[4]).map_err(|error| format!("read desired keys failed: {error}"))?,
    )
    .map_err(|error| format!("parse desired keys failed: {error}"))?
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    let source = MappedRenderObjectStore::open_metadata_catalog(&source_manifest)
        .map_err(|error| format!("open filter source failed: {error}"))?;
    let available = source
        .manifest()
        .objects
        .iter()
        .map(|object| object.key.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(missing) = desired.iter().find(|key| !available.contains(key.as_str())) {
        return Err(format!(
            "desired render object is absent after build: {missing}"
        ));
    }
    let mut identity = Sha256::new();
    identity.update(b"allium.filtered-render-object-store.v2");
    identity.update(source.manifest_sha256().as_bytes());
    for key in &desired {
        identity.update((key.len() as u64).to_le_bytes());
        identity.update(key.as_bytes());
    }
    let source_identity = hex::encode(identity.finalize());
    let output_dir = output_root.join(&source_identity);
    let manifest_path = output_dir.join("manifest.json");
    if manifest_path.is_file() {
        let store = MappedRenderObjectStore::open_metadata_catalog(&manifest_path)
            .map_err(|error| format!("open reused filtered store failed: {error}"))?;
        return print_simple_store_report(
            "allium.filtered-render-object-build.v1",
            true,
            &manifest_path,
            &store,
        );
    }
    if output_dir.exists() {
        return Err(format!(
            "filtered output exists without a valid manifest: {}",
            output_dir.display()
        ));
    }
    std::fs::create_dir_all(&output_dir)
        .map_err(|error| format!("create filtered output failed: {error}"))?;
    let mut objects = source
        .manifest()
        .objects
        .iter()
        .filter(|object| desired.contains(&object.key))
        .cloned()
        .collect::<Vec<_>>();
    let mut retained_pages = objects
        .iter()
        .map(|object| object.page)
        .collect::<std::collections::BTreeSet<_>>();
    // Empty filtered stores are used as an input to full Honor rebuilds. Keep
    // one immutable page so the existing non-empty page manifest contract holds.
    if retained_pages.is_empty() {
        retained_pages.insert(0);
    }
    let root = source_manifest.parent().unwrap_or_else(|| Path::new("."));
    let mut page_remap = BTreeMap::new();
    let mut pages = Vec::with_capacity(retained_pages.len());
    for old_index in retained_pages {
        let page = source
            .manifest()
            .pages
            .get(usize::from(old_index))
            .ok_or_else(|| format!("filtered page index is out of range: {old_index}"))?;
        let new_index = u16::try_from(pages.len())
            .map_err(|_| "filtered render-object page count exceeds u16".to_string())?;
        let file = format!("page-{new_index:04}.rgba");
        std::fs::hard_link(root.join(&page.file), output_dir.join(&file))
            .map_err(|error| format!("hardlink filtered page failed: {error}"))?;
        let mut page = page.clone();
        page.file = file;
        pages.push(page);
        page_remap.insert(old_index, new_index);
    }
    for object in &mut objects {
        object.page = *page_remap
            .get(&object.page)
            .ok_or_else(|| format!("filtered object page was not retained: {}", object.key))?;
    }
    let manifest = RenderObjectManifest {
        schema: source.manifest().schema.clone(),
        generator_contract: source.manifest().generator_contract.clone(),
        pixel_format: source.manifest().pixel_format.clone(),
        source_identity,
        pages,
        objects,
    };
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("serialize filtered manifest: {error}"))?,
    )
    .map_err(|error| format!("write filtered manifest failed: {error}"))?;
    let store = MappedRenderObjectStore::open_metadata_catalog(&manifest_path)
        .map_err(|error| format!("verify filtered store failed: {error}"))?;
    print_simple_store_report(
        "allium.filtered-render-object-build.v1",
        false,
        &manifest_path,
        &store,
    )
}

fn print_simple_store_report(
    schema: &str,
    reused: bool,
    manifest: &Path,
    store: &MappedRenderObjectStore,
) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": schema,
            "reused": reused,
            "manifest": manifest,
            "manifest_sha256": store.manifest_sha256(),
            "source_identity": store.manifest().source_identity,
            "object_count": store.manifest().objects.len(),
            "page_count": store.manifest().pages.len(),
        }))
        .map_err(|error| format!("serialize store report failed: {error}"))?
    );
    Ok(())
}

fn profile_warmth_class(key: &str) -> u8 {
    if key.starts_with("texture:assets/character/member_cutout/") {
        1
    } else if key.starts_with("texture:assets/thumbnail/chara/") {
        2
    } else if key.starts_with("texture:static/") {
        3
    } else if key.starts_with("texture:assets/character/member_small/") {
        4
    } else {
        0
    }
}

fn print_merge_report(
    reused: bool,
    source_identity: &str,
    manifest_path: &Path,
    store: &MappedRenderObjectStore,
    started: Instant,
) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "allium.merged-render-object-build.v1",
            "reused": reused,
            "source_identity": source_identity,
            "manifest": manifest_path,
            "manifest_sha256": store.manifest_sha256(),
            "object_count": store.manifest().objects.len(),
            "page_count": store.manifest().pages.len(),
            "mapped_bytes": store.mapped_bytes(),
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        }))
        .map_err(|error| format!("serialize merge report failed: {error}"))?
    );
    Ok(())
}

fn run_global_cache(args: &[String], build: bool) -> Result<(), String> {
    let (output_root, static_root, asset_cache_root, page_mib) = if build {
        if !(5..=6).contains(&args.len()) {
            return Err(format!(
                "usage: {} --global-cache <output-root> <static-root> <asset-cache-root> [page-mib]",
                args.first()
                    .map(String::as_str)
                    .unwrap_or("build-render-object-store")
            ));
        }
        (
            Some(PathBuf::from(&args[2])),
            PathBuf::from(&args[3]),
            PathBuf::from(&args[4]),
            args.get(5),
        )
    } else {
        if args.len() != 4 {
            return Err(format!(
                "usage: {} --global-cache-plan <static-root> <asset-cache-root>",
                args.first()
                    .map(String::as_str)
                    .unwrap_or("build-render-object-store")
            ));
        }
        (None, PathBuf::from(&args[2]), PathBuf::from(&args[3]), None)
    };
    let page_mib = page_mib
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|error| format!("invalid page-mib: {error}"))?
        .unwrap_or(DEFAULT_PAGE_MIB);
    let page_payload_limit = page_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| "page-mib overflow".to_string())?;
    let started = Instant::now();
    let sources = collect_global_sources(&static_root, &asset_cache_root)?;
    let estimated_pixel_bytes = sources
        .iter()
        .map(|source| source.pixel_bytes)
        .fold(0u64, u64::saturating_add);
    let mut identity = Sha256::new();
    identity.update(b"allium.global-render-object-store.v1");
    for source in &sources {
        identity.update((source.key.len() as u64).to_le_bytes());
        identity.update(source.key.as_bytes());
        identity.update(source.source_sha256.as_bytes());
        identity.update(source.width.to_le_bytes());
        identity.update(source.height.to_le_bytes());
    }
    let source_identity = hex::encode(identity.finalize());
    if !build {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": "allium.global-render-object-plan.v1",
                "source_identity": source_identity,
                "object_count": sources.len(),
                "estimated_pixel_bytes": estimated_pixel_bytes,
                "static_root": static_root,
                "asset_cache_root": asset_cache_root,
                "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
            }))
            .map_err(|error| format!("serialize global plan failed: {error}"))?
        );
        return Ok(());
    }

    let output_dir = output_root
        .expect("build mode has output root")
        .join(&source_identity);
    let manifest_path = output_dir.join("manifest.json");
    if manifest_path.is_file() {
        let store =
            MappedRenderObjectStore::open(&manifest_path).map_err(|error| error.to_string())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": "allium.global-render-object-build.v1",
                "reused": true,
                "source_identity": source_identity,
                "manifest": manifest_path,
                "manifest_sha256": store.manifest_sha256(),
                "object_count": store.manifest().objects.len(),
                "mapped_bytes": store.mapped_bytes(),
            }))
            .map_err(|error| format!("serialize reused global store failed: {error}"))?
        );
        return Ok(());
    }
    if output_dir.exists() {
        return Err(format!(
            "global store output exists without a valid manifest: {}",
            output_dir.display()
        ));
    }
    let mut writer = RenderObjectStoreWriter::create(
        &output_dir,
        format!("global-cache:{source_identity}"),
        page_payload_limit,
    )
    .map_err(|error| error.to_string())?;
    let mut decoded_bytes = 0u64;
    for source in &sources {
        let decoded = decode_premul_rgba8(&source.source_path, Some(&source.source_sha256))?;
        decoded_bytes = decoded_bytes.saturating_add(decoded.pixels.len() as u64);
        writer
            .add(RenderObjectWrite {
                key: &source.key,
                kind: RenderObjectKind::Texture,
                source_sha256: &decoded.source_sha256,
                width: decoded.width,
                height: decoded.height,
                row_bytes: decoded.row_bytes,
                pixels: &decoded.pixels,
            })
            .map_err(|error| error.to_string())?;
    }
    let manifest_path = writer.finish().map_err(|error| error.to_string())?;
    let store = MappedRenderObjectStore::open(&manifest_path).map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "allium.global-render-object-build.v1",
            "reused": false,
            "source_identity": source_identity,
            "manifest": manifest_path,
            "manifest_sha256": store.manifest_sha256(),
            "object_count": store.manifest().objects.len(),
            "page_count": store.manifest().pages.len(),
            "estimated_pixel_bytes": estimated_pixel_bytes,
            "decoded_bytes": decoded_bytes,
            "mapped_bytes": store.mapped_bytes(),
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        }))
        .map_err(|error| format!("serialize global build failed: {error}"))?
    );
    Ok(())
}

fn collect_global_sources(
    static_root: &Path,
    asset_cache_root: &Path,
) -> Result<Vec<GlobalObjectSource>, String> {
    let mut by_key = BTreeMap::<String, PathBuf>::new();
    for path in recursive_files(static_root)? {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("png" | "jpg" | "jpeg")) {
            continue;
        }
        let relative = path
            .strip_prefix(static_root)
            .map_err(|error| format!("strip static prefix failed: {error}"))?;
        let mut key = relative
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        key.insert_str(0, "texture:static/");
        insert_global_source(&mut by_key, key, path)?;
    }
    for path in recursive_files(asset_cache_root)? {
        let relative = path
            .strip_prefix(asset_cache_root)
            .map_err(|error| format!("strip asset cache prefix failed: {error}"))?;
        let key = relative
            .to_string_lossy()
            .replace('\\', "/")
            .replace("__", "/");
        insert_global_source(&mut by_key, format!("texture:assets/{key}"), path)?;
    }

    by_key
        .into_iter()
        .map(|(key, source_path)| inspect_global_source(key, source_path))
        .collect()
}

fn insert_global_source(
    output: &mut BTreeMap<String, PathBuf>,
    key: String,
    path: PathBuf,
) -> Result<(), String> {
    if let Some(existing) = output.insert(key.clone(), path.clone()) {
        return Err(format!(
            "duplicate global resource key {key}: {} and {}",
            existing.display(),
            path.display()
        ));
    }
    Ok(())
}

fn recursive_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| format!("read directory {} failed: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| {
                format!(
                    "read directory entry {} failed: {error}",
                    directory.display()
                )
            })?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("read file type failed: {error}"))?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                files.push(entry.path());
            }
        }
    }
    files.sort_unstable();
    Ok(files)
}

fn inspect_global_source(key: String, source_path: PathBuf) -> Result<GlobalObjectSource, String> {
    let encoded = std::fs::read(&source_path)
        .map_err(|error| format!("read {} failed: {error}", source_path.display()))?;
    let source_sha256 = hex::encode(Sha256::digest(&encoded));
    let image = Image::from_encoded(Data::new_copy(&encoded))
        .ok_or_else(|| format!("decode image metadata {} failed", source_path.display()))?;
    let width = u32::try_from(image.width())
        .map_err(|_| format!("negative image width for {}", source_path.display()))?;
    let height = u32::try_from(image.height())
        .map_err(|_| format!("negative image height for {}", source_path.display()))?;
    let pixel_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| format!("pixel byte overflow for {}", source_path.display()))?;
    Ok(GlobalObjectSource {
        key,
        source_path,
        source_sha256,
        width,
        height,
        pixel_bytes,
    })
}

fn run_compiled_report(args: &[String]) -> Result<(), String> {
    if !(6..=7).contains(&args.len()) {
        return Err(format!(
            "usage: {} --compiled-report <report.json> <output-dir> <static-root> <asset-cache-root> [page-mib]",
            args.first()
                .map(String::as_str)
                .unwrap_or("build-render-object-store")
        ));
    }
    let report_path = PathBuf::from(&args[2]);
    let output_dir = PathBuf::from(&args[3]);
    let static_root = PathBuf::from(&args[4]);
    let asset_cache_root = PathBuf::from(&args[5]);
    let page_mib = args
        .get(6)
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|error| format!("invalid page-mib: {error}"))?
        .unwrap_or(DEFAULT_PAGE_MIB);
    let page_payload_limit = page_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| "page-mib overflow".to_string())?;
    let started = Instant::now();
    let report: CompiledBatchReport = serde_json::from_slice(
        &std::fs::read(&report_path)
            .map_err(|error| format!("read {} failed: {error}", report_path.display()))?,
    )
    .map_err(|error| format!("parse {} failed: {error}", report_path.display()))?;
    let unique = report
        .batch
        .resources
        .iter()
        .map(|resource| (resource.render_object_key.clone(), resource))
        .collect::<BTreeMap<_, _>>();
    let resolved = resolve_compiled_resource_sources(&unique, &static_root, &asset_cache_root)?;
    let mut writer = RenderObjectStoreWriter::create(
        &output_dir,
        format!("compiled-profile-batch:{}", report.batch.identity),
        page_payload_limit,
    )
    .map_err(|error| error.to_string())?;
    let mut decoded_bytes = 0u64;
    let mut source_bytes = 0u64;
    for (render_object_key, _resource, source_path) in resolved {
        let encoded_len = std::fs::metadata(&source_path)
            .map_err(|error| format!("metadata {} failed: {error}", source_path.display()))?
            .len();
        let decoded = decode_premul_rgba8(&source_path, None)?;
        source_bytes = source_bytes.saturating_add(encoded_len);
        decoded_bytes = decoded_bytes.saturating_add(decoded.pixels.len() as u64);
        writer
            .add(RenderObjectWrite {
                key: &render_object_key,
                kind: RenderObjectKind::Texture,
                source_sha256: &decoded.source_sha256,
                width: decoded.width,
                height: decoded.height,
                row_bytes: decoded.row_bytes,
                pixels: &decoded.pixels,
            })
            .map_err(|error| error.to_string())?;
    }
    let manifest_path = writer.finish().map_err(|error| error.to_string())?;
    let store = MappedRenderObjectStore::open(&manifest_path).map_err(|error| error.to_string())?;
    let preparation = report.batch.prepare_render_objects(&store);
    if !preparation.missing_object_keys.is_empty() {
        return Err(format!(
            "generated store is missing {} compiled objects",
            preparation.missing_object_keys.len()
        ));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "allium.render-object-build-report.v1",
            "compiled_report": report_path,
            "compiled_batch_identity": report.batch.identity,
            "manifest": manifest_path,
            "manifest_sha256": store.manifest_sha256(),
            "source_identity": store.manifest().source_identity,
            "object_count": store.manifest().objects.len(),
            "page_count": store.manifest().pages.len(),
            "source_bytes": source_bytes,
            "decoded_bytes": decoded_bytes,
            "mapped_bytes": store.mapped_bytes(),
            "preparation": preparation,
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        }))
        .map_err(|error| format!("serialize build report failed: {error}"))?
    );
    Ok(())
}

fn resolve_source_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "source path must be a normal relative path: {relative}"
        ));
    }
    Ok(root.join(path))
}

fn resolve_compiled_resource_source(
    resource: &CompiledResourceRequest,
    static_root: &Path,
    asset_cache_root: &Path,
) -> Result<PathBuf, String> {
    let candidates = match resource.namespace.as_str() {
        "static" => image_candidates(static_root.join(&resource.key)),
        "assets" => {
            let normalized = resource.key.replace('/', "__");
            let mut candidates = vec![asset_cache_root.join(normalized)];
            candidates.extend(image_candidates(asset_cache_root.join(&resource.key)));
            candidates
        }
        namespace => {
            return Err(format!(
                "unsupported compiled resource namespace {namespace} for {}",
                resource.render_object_key
            ));
        }
    };
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "source asset not found for {} ({}/{})",
                resource.render_object_key, resource.namespace, resource.key
            )
        })
}

fn resolve_compiled_resource_sources<'a>(
    resources: &'a BTreeMap<String, &'a CompiledResourceRequest>,
    static_root: &Path,
    asset_cache_root: &Path,
) -> Result<Vec<(&'a str, &'a CompiledResourceRequest, PathBuf)>, String> {
    let mut resolved = Vec::with_capacity(resources.len());
    let mut missing = Vec::new();
    for (render_object_key, resource) in resources {
        match resolve_compiled_resource_source(resource, static_root, asset_cache_root) {
            Ok(source_path) => {
                resolved.push((render_object_key.as_str(), *resource, source_path));
            }
            Err(error) => missing.push(error),
        }
    }
    if missing.is_empty() {
        Ok(resolved)
    } else {
        Err(format!(
            "compiled resource preflight failed for {} object(s):\n{}",
            missing.len(),
            missing.join("\n")
        ))
    }
}

fn image_candidates(base: PathBuf) -> Vec<PathBuf> {
    ["png", "jpg", "jpeg"]
        .into_iter()
        .map(|extension| base.with_extension(extension))
        .collect()
}

fn decode_premul_rgba8(
    path: &Path,
    expected_sha256: Option<&str>,
) -> Result<DecodedObject, String> {
    let encoded =
        std::fs::read(path).map_err(|error| format!("read {} failed: {error}", path.display()))?;
    let source_sha256 = hex::encode(Sha256::digest(&encoded));
    if expected_sha256.is_some_and(|expected| expected != source_sha256) {
        return Err(format!(
            "source hash mismatch for {}: expected {}, got {}",
            path.display(),
            expected_sha256.unwrap_or_default(),
            source_sha256
        ));
    }
    let image = Image::from_encoded(Data::new_copy(&encoded))
        .ok_or_else(|| format!("decode {} failed", path.display()))?;
    let width = u32::try_from(image.width())
        .map_err(|_| format!("negative image width for {}", path.display()))?;
    let height = u32::try_from(image.height())
        .map_err(|_| format!("negative image height for {}", path.display()))?;
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| format!("row_bytes overflow for {}", path.display()))?;
    let len = row_bytes
        .checked_mul(height)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| format!("pixel length overflow for {}", path.display()))?;
    let mut pixels = vec![0u8; len];
    let info = ImageInfo::new(
        (image.width(), image.height()),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    if !image.read_pixels(
        &info,
        &mut pixels,
        row_bytes as usize,
        (0, 0),
        skia_safe::image::CachingHint::Allow,
    ) {
        return Err(format!("read premul pixels {} failed", path.display()));
    }
    Ok(DecodedObject {
        source_sha256,
        width,
        height,
        row_bytes,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_store(root: &Path, name: &str, key: &str, value: u8) -> PathBuf {
        write_test_store_with_keys(root, name, &[key], value)
    }

    fn write_test_store_with_keys(root: &Path, name: &str, keys: &[&str], value: u8) -> PathBuf {
        let output = root.join(name);
        let pixels = [value; 16];
        let source = hex::encode(Sha256::digest([value]));
        let mut writer =
            RenderObjectStoreWriter::create(&output, name, 64).expect("create test store");
        let mut sorted_keys = keys.to_vec();
        sorted_keys.sort_unstable();
        for key in sorted_keys {
            writer
                .add(RenderObjectWrite {
                    key,
                    kind: RenderObjectKind::Texture,
                    source_sha256: &source,
                    width: 2,
                    height: 2,
                    row_bytes: 8,
                    pixels: &pixels,
                })
                .expect("add test object");
        }
        writer.finish().expect("finish test store")
    }

    #[test]
    fn source_paths_are_confined_to_build_list_directory() {
        let root = Path::new("fixture");
        assert_eq!(
            resolve_source_path(root, "honor/frame.png").expect("valid path"),
            root.join("honor/frame.png")
        );
        assert!(resolve_source_path(root, "../frame.png").is_err());
        assert!(resolve_source_path(root, "/frame.png").is_err());
    }

    #[test]
    fn compiled_resources_resolve_static_and_normalized_cache_paths() {
        let root = tempfile::tempdir().expect("tempdir");
        let static_root = root.path().join("static");
        let cache_root = root.path().join("cache");
        std::fs::create_dir_all(static_root.join("honor")).expect("static dir");
        std::fs::create_dir_all(&cache_root).expect("cache dir");
        std::fs::write(static_root.join("honor/frame.png"), b"static").expect("static file");
        std::fs::write(cache_root.join("honor__dynamic"), b"asset").expect("cache file");

        let static_resource = CompiledResourceRequest {
            namespace: "static".into(),
            key: "honor/frame".into(),
            use_kind: allium_renderer::compiled_profile::CompiledResourceUseKind::Image,
            render_object_key: "texture:static/honor/frame".into(),
        };
        let asset_resource = CompiledResourceRequest {
            namespace: "assets".into(),
            key: "honor/dynamic".into(),
            use_kind: allium_renderer::compiled_profile::CompiledResourceUseKind::Image,
            render_object_key: "texture:assets/honor/dynamic".into(),
        };
        assert_eq!(
            resolve_compiled_resource_source(&static_resource, &static_root, &cache_root)
                .expect("static source"),
            static_root.join("honor/frame.png")
        );
        assert_eq!(
            resolve_compiled_resource_source(&asset_resource, &static_root, &cache_root)
                .expect("cached source"),
            cache_root.join("honor__dynamic")
        );
    }

    #[test]
    fn compiled_resource_preflight_reports_every_missing_source() {
        let root = tempfile::tempdir().expect("tempdir");
        let static_root = root.path().join("static");
        let cache_root = root.path().join("cache");
        std::fs::create_dir_all(&static_root).expect("static dir");
        std::fs::create_dir_all(&cache_root).expect("cache dir");
        let first = CompiledResourceRequest {
            namespace: "assets".into(),
            key: "honor/missing-a".into(),
            use_kind: allium_renderer::compiled_profile::CompiledResourceUseKind::Image,
            render_object_key: "texture:assets/honor/missing-a".into(),
        };
        let second = CompiledResourceRequest {
            namespace: "assets".into(),
            key: "stamp/missing-b".into(),
            use_kind: allium_renderer::compiled_profile::CompiledResourceUseKind::Image,
            render_object_key: "texture:assets/stamp/missing-b".into(),
        };
        let fixtures = [first, second];
        let resources = fixtures
            .iter()
            .map(|resource| (resource.render_object_key.clone(), resource))
            .collect::<BTreeMap<_, _>>();

        let error = resolve_compiled_resource_sources(&resources, &static_root, &cache_root)
            .expect_err("missing sources must fail together");
        assert!(error.contains("2 object(s)"));
        assert!(error.contains("texture:assets/honor/missing-a"));
        assert!(error.contains("texture:assets/stamp/missing-b"));
    }

    #[test]
    fn merged_store_reuses_pages_and_offsets_delta_page_indices() {
        let root = tempfile::tempdir().expect("tempdir");
        let base_manifest_path = write_test_store(root.path(), "base", "texture:assets/base", 1);
        let delta_manifest_path = write_test_store(root.path(), "delta", "texture:assets/delta", 2);
        let base = MappedRenderObjectStore::open(&base_manifest_path).expect("base store");
        let delta = MappedRenderObjectStore::open(&delta_manifest_path).expect("delta store");
        let output = root.path().join("merged");
        std::fs::create_dir(&output).expect("merged directory");
        let manifest = merge_store_manifests(
            &output,
            &base_manifest_path,
            base.manifest(),
            &delta_manifest_path,
            delta.manifest(),
            "merged-fixture".into(),
        )
        .expect("merge stores");
        assert_eq!(manifest.pages.len(), 2);
        assert_eq!(manifest.objects.len(), 2);
        assert_eq!(manifest.objects[0].page, 0);
        assert_eq!(manifest.objects[1].page, 1);
        let merged_manifest_path = output.join("manifest.json");
        std::fs::write(
            &merged_manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        let merged =
            MappedRenderObjectStore::open(&merged_manifest_path).expect("open merged store");
        assert_eq!(
            merged.object("texture:assets/base").expect("base").pixels,
            [1; 16]
        );
        assert_eq!(
            merged.object("texture:assets/delta").expect("delta").pixels,
            [2; 16]
        );
    }

    #[test]
    fn merged_store_replaces_duplicate_keys_without_copying_base_pages() {
        let root = tempfile::tempdir().expect("tempdir");
        let base_manifest_path = write_test_store(root.path(), "base", "texture:assets/same", 1);
        let delta_manifest_path = write_test_store(root.path(), "delta", "texture:assets/same", 2);
        let base = MappedRenderObjectStore::open(&base_manifest_path).expect("base store");
        let delta = MappedRenderObjectStore::open(&delta_manifest_path).expect("delta store");
        let output = root.path().join("merged");
        std::fs::create_dir(&output).expect("merged directory");
        let manifest = merge_store_manifests(
            &output,
            &base_manifest_path,
            base.manifest(),
            &delta_manifest_path,
            delta.manifest(),
            "replacement-fixture".into(),
        )
        .expect("merge replacement");
        assert_eq!(manifest.pages.len(), 2);
        assert_eq!(manifest.objects.len(), 1);
        assert_eq!(manifest.objects[0].page, 1);
        let manifest_path = output.join("manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        let merged = MappedRenderObjectStore::open(&manifest_path).expect("open merged store");
        assert_eq!(
            merged
                .object("texture:assets/same")
                .expect("replacement")
                .pixels,
            [2; 16]
        );
        let base_inode = std::fs::metadata(
            base_manifest_path
                .parent()
                .expect("base root")
                .join("page-0000.rgba"),
        )
        .expect("base page metadata");
        let merged_inode =
            std::fs::metadata(output.join("page-0000.rgba")).expect("merged page metadata");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(base_inode.ino(), merged_inode.ino());
        }
    }

    #[test]
    fn filtered_store_drops_unreferenced_pages_and_remaps_objects() {
        let root = tempfile::tempdir().expect("tempdir");
        let source_manifest = write_test_store_with_keys(
            root.path(),
            "source",
            &["texture:assets/a", "texture:assets/b"],
            7,
        );
        let desired_path = root.path().join("desired.json");
        std::fs::write(
            &desired_path,
            serde_json::to_vec(&vec!["texture:assets/b"]).expect("desired keys"),
        )
        .expect("write desired keys");
        let output_root = root.path().join("filtered");
        run_filter_store(&[
            "build-render-object-store".into(),
            "--filter-store".into(),
            output_root.to_string_lossy().into_owned(),
            source_manifest.to_string_lossy().into_owned(),
            desired_path.to_string_lossy().into_owned(),
        ])
        .expect("filter store");

        let output_dir = std::fs::read_dir(&output_root)
            .expect("filtered output root")
            .next()
            .expect("filtered generation")
            .expect("filtered generation entry")
            .path();
        let filtered = MappedRenderObjectStore::open(output_dir.join("manifest.json"))
            .expect("open filtered store");
        assert_eq!(filtered.manifest().pages.len(), 1);
        assert_eq!(filtered.manifest().objects.len(), 1);
        assert_eq!(filtered.manifest().objects[0].key, "texture:assets/b");
        assert_eq!(filtered.manifest().objects[0].page, 0);
        assert_eq!(
            filtered
                .object("texture:assets/b")
                .expect("retained object")
                .pixels,
            [7; 16]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let source_page = std::fs::metadata(
                source_manifest
                    .parent()
                    .expect("source root")
                    .join("page-0001.rgba"),
            )
            .expect("source page");
            let filtered_page =
                std::fs::metadata(output_dir.join("page-0000.rgba")).expect("filtered page");
            assert_eq!(source_page.ino(), filtered_page.ino());
        }
    }

    #[test]
    fn global_warmth_order_keeps_dense_card_objects_at_the_tail() {
        assert!(
            profile_warmth_class("texture:assets/character/member_small/res/card_normal")
                > profile_warmth_class("texture:assets/honor/example/degree_main")
        );
        assert!(
            profile_warmth_class("texture:assets/character/member_small/res/card_normal")
                > profile_warmth_class("texture:static/honor/frame")
        );
    }

    #[test]
    fn desired_catalog_planner_adds_new_texture_and_deck_variant_without_profiles() {
        let root = tempfile::tempdir().expect("tempdir");
        let current_manifest = write_test_store_with_keys(
            root.path(),
            "current",
            &[
                "texture:static/ui/fixed",
                "texture:assets/character/member_cutout/res001/card_normal",
                "texture:assets/character/member_cutout/res002/card_normal",
            ],
            7,
        );
        let masterdata = root.path().join("masterdata");
        std::fs::create_dir(&masterdata).expect("masterdata dir");
        for table in [
            "honors",
            "honorGroups",
            "bondsHonors",
            "bondsHonorWords",
            "gameCharacterUnits",
        ] {
            std::fs::write(masterdata.join(format!("{table}.json")), b"[]")
                .expect("masterdata table");
        }
        let sha_a = "a".repeat(64);
        let sha_b = hex::encode(Sha256::digest([7]));
        let header = DesiredRenderObjectCatalog {
            schema: allium_renderer::render_object_catalog::DESIRED_RENDER_OBJECT_CATALOG_SCHEMA
                .into(),
            region: "cn".into(),
            data_version: "v1".into(),
            index_revision: 1,
            asset_index_manifest_key: "asset-index/v1/manifest.json".into(),
            asset_index_manifest_sha256: sha_a.clone(),
            masterdata_object_key: "masterdata/cn/v1/masterdata.json".into(),
            masterdata_sha256: sha_b.clone(),
            recipe_set_contract: "recipes-v1".into(),
            builder_static_identity: sha_a.clone(),
            atlas_identities: vec![],
            build_dependencies: vec![],
            objects: vec![],
            catalog_sha256: String::new(),
        };
        let header_path = root.path().join("header.json");
        std::fs::write(
            &header_path,
            serde_json::to_vec(&header).expect("header json"),
        )
        .expect("header");
        let assets_path = root.path().join("assets.json");
        std::fs::write(
            &assets_path,
            serde_json::to_vec(&vec![
                serde_json::json!({
                    "logical_path": "character/member_cutout/res001/card_normal.png",
                    "object_key": format!("asset-blobs/sha256/bb/{sha_b}"),
                    "sha256": sha_b,
                }),
                serde_json::json!({
                    "logical_path": "character/member_cutout/res002/card_normal.png",
                    "object_key": format!("asset-blobs/sha256/bb/{sha_b}"),
                    "sha256": sha_b,
                }),
            ])
            .expect("assets json"),
        )
        .expect("assets");
        let output = root.path().join("catalog.json");
        let args = vec![
            "build-render-object-store".into(),
            "--plan-desired-catalog".into(),
            header_path.to_string_lossy().into_owned(),
            masterdata.to_string_lossy().into_owned(),
            assets_path.to_string_lossy().into_owned(),
            current_manifest.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
        ];
        run_plan_desired_catalog(&args).expect("plan desired catalog");
        let catalog: DesiredRenderObjectCatalog =
            serde_json::from_slice(&std::fs::read(output).expect("catalog bytes"))
                .expect("catalog");
        assert!(catalog
            .objects
            .iter()
            .any(|object| object.key == "texture:static/ui/fixed"));
        assert!(catalog.objects.iter().any(|object| {
            object.key == "texture:assets/character/member_cutout/res001/card_normal"
        }));
        assert!(catalog.objects.iter().any(|object| {
            object.key == "texture:assets/character/member_cutout/res002/card_normal"
        }));
        assert_eq!(
            catalog
                .objects
                .iter()
                .filter(|object| object.key.starts_with("component:deck-art-variant/"))
                .count(),
            1
        );
    }
}
