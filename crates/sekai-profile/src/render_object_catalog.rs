//! Desired-state contract for resource-pipeline render-object generations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::render_object::RenderObjectManifest;

pub const DESIRED_RENDER_OBJECT_CATALOG_SCHEMA: &str = "allium.render-object-desired-catalog.v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderObjectSourceIdentity {
    pub name: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct RenderObjectDependency {
    pub kind: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub logical_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub object_key: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DesiredRenderObject {
    pub key: String,
    pub kind: String,
    pub recipe_contract: String,
    pub recipe_sha256: String,
    #[serde(default)]
    pub source_identity: String,
    #[serde(default)]
    pub dependencies: Vec<RenderObjectDependency>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DesiredRenderObjectCatalog {
    pub schema: String,
    pub region: String,
    pub data_version: String,
    pub index_revision: i64,
    pub asset_index_manifest_key: String,
    pub asset_index_manifest_sha256: String,
    pub masterdata_object_key: String,
    pub masterdata_sha256: String,
    pub recipe_set_contract: String,
    pub builder_static_identity: String,
    #[serde(default)]
    pub atlas_identities: Vec<RenderObjectSourceIdentity>,
    #[serde(default)]
    pub build_dependencies: Vec<RenderObjectDependency>,
    pub objects: Vec<DesiredRenderObject>,
    #[serde(default)]
    pub catalog_sha256: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderObjectCatalogDiff {
    pub reuse: Vec<String>,
    pub build: Vec<String>,
    pub remove: Vec<String>,
}

impl DesiredRenderObjectCatalog {
    pub fn seal(mut self) -> Result<Self, String> {
        if self.schema != DESIRED_RENDER_OBJECT_CATALOG_SCHEMA {
            return Err(format!(
                "unsupported desired catalog schema {}",
                self.schema
            ));
        }
        if !matches!(self.region.as_str(), "cn" | "jp")
            || self.data_version.trim().is_empty()
            || self.index_revision <= 0
        {
            return Err("desired catalog has an invalid release identity".into());
        }
        for (name, value) in [
            ("asset index manifest key", &self.asset_index_manifest_key),
            ("masterdata object key", &self.masterdata_object_key),
            ("recipe set contract", &self.recipe_set_contract),
            ("builder static identity", &self.builder_static_identity),
        ] {
            if value.trim().is_empty() {
                return Err(format!("desired catalog {name} is empty"));
            }
        }
        for (name, value) in [
            (
                "asset index manifest SHA-256",
                &self.asset_index_manifest_sha256,
            ),
            ("masterdata SHA-256", &self.masterdata_sha256),
            ("builder static identity", &self.builder_static_identity),
        ] {
            if !is_sha256(value) {
                return Err(format!("desired catalog {name} is invalid"));
            }
        }

        self.atlas_identities
            .sort_unstable_by(|left, right| left.name.cmp(&right.name));
        for (index, identity) in self.atlas_identities.iter().enumerate() {
            if identity.name.trim().is_empty() || !is_sha256(&identity.sha256) {
                return Err(format!("invalid atlas identity at index {index}"));
            }
            if index > 0 && self.atlas_identities[index - 1].name == identity.name {
                return Err(format!("duplicate atlas identity {}", identity.name));
            }
        }
        self.build_dependencies.sort_unstable();
        for index in 0..self.build_dependencies.len() {
            validate_dependency(&self.build_dependencies[index], "catalog build dependency")?;
            if index > 0
                && self.build_dependencies[index - 1].kind == self.build_dependencies[index].kind
                && self.build_dependencies[index - 1].key == self.build_dependencies[index].key
            {
                return Err(format!(
                    "duplicate catalog build dependency {}:{}",
                    self.build_dependencies[index].kind, self.build_dependencies[index].key
                ));
            }
        }

        self.objects
            .sort_unstable_by(|left, right| left.key.cmp(&right.key));
        for index in 0..self.objects.len() {
            if index > 0 && self.objects[index - 1].key == self.objects[index].key {
                return Err(format!(
                    "duplicate desired render object {}",
                    self.objects[index].key
                ));
            }
            let identity = desired_object_identity(&mut self.objects[index])?;
            if !self.objects[index].source_identity.is_empty()
                && self.objects[index].source_identity != identity
            {
                return Err(format!(
                    "source identity mismatch for {}",
                    self.objects[index].key
                ));
            }
            self.objects[index].source_identity = identity;
        }
        let identity = desired_catalog_identity(&self);
        if !self.catalog_sha256.is_empty() && self.catalog_sha256 != identity {
            return Err("desired catalog SHA-256 mismatch".into());
        }
        self.catalog_sha256 = identity;
        Ok(self)
    }

    pub fn diff_against(
        &self,
        current: &RenderObjectManifest,
    ) -> Result<RenderObjectCatalogDiff, String> {
        let sealed = self.clone().seal()?;
        let mut current_by_key = current
            .objects
            .iter()
            .map(|object| (object.key.clone(), object.source_sha256.clone()))
            .collect::<BTreeMap<_, _>>();
        if current_by_key.len() != current.objects.len() {
            return Err("current render-object manifest contains duplicate keys".into());
        }
        let mut diff = RenderObjectCatalogDiff::default();
        for object in sealed.objects {
            if current_by_key.get(&object.key) == Some(&object.source_identity) {
                diff.reuse.push(object.key.clone());
            } else {
                diff.build.push(object.key.clone());
            }
            current_by_key.remove(&object.key);
        }
        diff.remove.extend(current_by_key.into_keys());
        Ok(diff)
    }
}

fn desired_object_identity(object: &mut DesiredRenderObject) -> Result<String, String> {
    if object.key.trim().is_empty() || object.recipe_contract.trim().is_empty() {
        return Err("desired render object key or recipe contract is empty".into());
    }
    if !matches!(
        object.kind.as_str(),
        "texture" | "standard_honor" | "bonds_honor" | "component"
    ) {
        return Err(format!(
            "unsupported render object kind {} for {}",
            object.kind, object.key
        ));
    }
    if !is_sha256(&object.recipe_sha256) {
        return Err(format!("invalid recipe SHA-256 for {}", object.key));
    }
    object.dependencies.sort_unstable();
    for (index, dependency) in object.dependencies.iter().enumerate() {
        validate_dependency(dependency, &object.key)?;
        if index > 0
            && object.dependencies[index - 1].kind == dependency.kind
            && object.dependencies[index - 1].key == dependency.key
        {
            return Err(format!(
                "duplicate dependency {}:{} for {}",
                dependency.kind, dependency.key, object.key
            ));
        }
    }
    if object.recipe_contract == "allium.render-object.prebuilt.v1" {
        if object.dependencies.len() != 1
            || !matches!(
                object.dependencies[0].kind.as_str(),
                "builder_static" | "asset_blob"
            )
        {
            return Err(format!(
                "prebuilt render object {} must have one immutable dependency",
                object.key
            ));
        }
        return Ok(object.dependencies[0].sha256.clone());
    }
    let mut digest = Sha256::new();
    for value in [
        "allium.render-object-source-identity.v1",
        object.key.as_str(),
        object.kind.as_str(),
        object.recipe_contract.as_str(),
        object.recipe_sha256.as_str(),
    ] {
        hash_field(&mut digest, value);
    }
    for dependency in &object.dependencies {
        for value in [
            dependency.kind.as_str(),
            dependency.key.as_str(),
            dependency.logical_path.as_str(),
            dependency.object_key.as_str(),
            dependency.sha256.as_str(),
        ] {
            hash_field(&mut digest, value);
        }
    }
    Ok(hex::encode(digest.finalize()))
}

fn validate_dependency(dependency: &RenderObjectDependency, owner: &str) -> Result<(), String> {
    if dependency.key.trim().is_empty() || !is_sha256(&dependency.sha256) {
        return Err(format!("invalid dependency identity for {owner}"));
    }
    match dependency.kind.as_str() {
        "asset_blob" => {
            if dependency.logical_path.trim().is_empty() || dependency.object_key.trim().is_empty()
            {
                return Err(format!(
                    "asset blob dependency {} for {owner} is incomplete",
                    dependency.key
                ));
            }
        }
        "masterdata" | "builder_static" | "atlas" => {}
        value => return Err(format!("unsupported dependency kind {value} for {owner}")),
    }
    Ok(())
}

fn desired_catalog_identity(catalog: &DesiredRenderObjectCatalog) -> String {
    let mut digest = Sha256::new();
    for value in [
        DESIRED_RENDER_OBJECT_CATALOG_SCHEMA,
        catalog.region.as_str(),
        catalog.data_version.as_str(),
    ] {
        hash_field(&mut digest, value);
    }
    digest.update(catalog.index_revision.to_le_bytes());
    for value in [
        catalog.asset_index_manifest_key.as_str(),
        catalog.asset_index_manifest_sha256.as_str(),
        catalog.masterdata_object_key.as_str(),
        catalog.masterdata_sha256.as_str(),
        catalog.recipe_set_contract.as_str(),
        catalog.builder_static_identity.as_str(),
    ] {
        hash_field(&mut digest, value);
    }
    for identity in &catalog.atlas_identities {
        hash_field(&mut digest, &identity.name);
        hash_field(&mut digest, &identity.sha256);
    }
    for dependency in &catalog.build_dependencies {
        for value in [
            dependency.kind.as_str(),
            dependency.key.as_str(),
            dependency.logical_path.as_str(),
            dependency.object_key.as_str(),
            dependency.sha256.as_str(),
        ] {
            hash_field(&mut digest, value);
        }
    }
    for object in &catalog.objects {
        hash_field(&mut digest, &object.key);
        hash_field(&mut digest, &object.source_identity);
    }
    hex::encode(digest.finalize())
}

fn hash_field(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value.as_bytes());
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_object::{RenderObjectEntry, RenderObjectKind};

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn catalog() -> DesiredRenderObjectCatalog {
        DesiredRenderObjectCatalog {
            schema: DESIRED_RENDER_OBJECT_CATALOG_SCHEMA.into(),
            region: "cn".into(),
            data_version: "v1".into(),
            index_revision: 3,
            asset_index_manifest_key: "asset-index/snapshots/v1/manifest.json".into(),
            asset_index_manifest_sha256: SHA_A.into(),
            masterdata_object_key: "masterdata/cn/v1/masterdata.json".into(),
            masterdata_sha256: SHA_B.into(),
            recipe_set_contract: "renderer-recipes-v1".into(),
            builder_static_identity: SHA_A.into(),
            atlas_identities: vec![],
            build_dependencies: vec![],
            objects: vec![DesiredRenderObject {
                key: "texture:assets/a".into(),
                kind: "texture".into(),
                recipe_contract: "texture-rgba-v1".into(),
                recipe_sha256: SHA_A.into(),
                source_identity: String::new(),
                dependencies: vec![RenderObjectDependency {
                    kind: "asset_blob".into(),
                    key: "asset:a.png".into(),
                    logical_path: "a.png".into(),
                    object_key: format!("asset-blobs/sha256/bb/{SHA_B}"),
                    sha256: SHA_B.into(),
                }],
            }],
            catalog_sha256: String::new(),
        }
    }

    #[test]
    fn seals_and_diffs_by_key_and_source_identity() {
        let sealed = catalog().seal().expect("seal catalog");
        assert_eq!(
            sealed.objects[0].source_identity,
            "e3cabba56134b2d412fdca3bc70efd92c9396da093e0827cda50fb5bc0d17474"
        );
        assert_eq!(
            sealed.catalog_sha256,
            "c2aa8a087afbe2c7413e5b8588d03d37e849045e94ff8080429ae164b0f3dbc6"
        );
        let mut current = RenderObjectManifest {
            schema: "schema".into(),
            generator_contract: "generator".into(),
            pixel_format: "rgba".into(),
            source_identity: "current".into(),
            pages: vec![],
            objects: vec![RenderObjectEntry {
                key: sealed.objects[0].key.clone(),
                kind: RenderObjectKind::Texture,
                source_sha256: sealed.objects[0].source_identity.clone(),
                page: 0,
                offset: 0,
                length: 4,
                width: 1,
                height: 1,
                row_bytes: 4,
                pixel_sha256: SHA_A.into(),
            }],
        };
        let diff = sealed.diff_against(&current).expect("diff catalog");
        assert_eq!(diff.reuse, vec!["texture:assets/a"]);
        assert!(diff.build.is_empty());
        assert!(diff.remove.is_empty());

        current.objects[0].source_sha256 = SHA_B.into();
        let diff = sealed.diff_against(&current).expect("diff changed catalog");
        assert_eq!(diff.build, vec!["texture:assets/a"]);
    }
}
