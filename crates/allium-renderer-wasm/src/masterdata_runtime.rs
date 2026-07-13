use std::cell::RefCell;
use std::collections::BTreeMap;

use allium_renderer_core::masterdata::{JsonMasterData, PROFILE_MASTERDATA_TABLES};
use allium_renderer_core::profile_data::ProfileData;
use allium_renderer_core::profile_resolve::{
    compile_profile_scene, prepare_profile, ResourceMetadata, ResourceMetric,
};
use allium_renderer_core::profile_source::CustomProfileCard;
use allium_renderer_core::{LineIndentSource, ResourceKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

thread_local! {
    static MASTERDATA: RefCell<MasterDataTable> = RefCell::new(MasterDataTable::default());
}

#[derive(Default)]
struct MasterDataTable {
    next_handle: u32,
    sessions: BTreeMap<u32, MasterDataSession>,
}

struct MasterDataSession {
    region: String,
    revision: String,
    data: JsonMasterData,
    sealed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    region: String,
    revision: String,
}

#[derive(Serialize)]
struct CreateResponse {
    handle: u32,
    region: String,
    revision: String,
    required_tables: &'static [&'static str],
}

#[derive(Deserialize)]
struct PutTableRequest {
    name: String,
    table: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareRequest {
    document_key: String,
    card: CustomProfileCard,
    #[serde(default)]
    profile: Option<Value>,
    #[serde(default)]
    locale: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompileRequest {
    document_key: String,
    card: CustomProfileCard,
    #[serde(default)]
    profile: Option<Value>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    resource_metrics: Vec<ResourceMetricInput>,
    #[serde(default)]
    dynamic_programs: Vec<DynamicProgramInput>,
    #[serde(default)]
    frame_mode: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DynamicProgramInput {
    layer_id: String,
    percent: f32,
    advances_tmp: Vec<f32>,
    rotation_deg: f32,
    scale_x: f32,
}

#[derive(Deserialize)]
struct ResourceMetricInput {
    namespace: String,
    key: String,
    width: f32,
    height: f32,
}

struct ResourceMetricMap(BTreeMap<(String, String), ResourceMetric>);

impl ResourceMetadata for ResourceMetricMap {
    fn metric(&self, resource: &ResourceKey) -> Option<ResourceMetric> {
        self.0
            .get(&(resource.namespace.clone(), resource.key.clone()))
            .copied()
    }
}

#[derive(Serialize)]
struct SessionStats<'a> {
    handle: u32,
    region: &'a str,
    revision: &'a str,
    sealed: bool,
    tables: Vec<&'a str>,
}

pub fn create(input: &str) -> Result<String, String> {
    let request: CreateRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse master-data session failed: {error}"))?;
    if request.region.is_empty() || request.revision.is_empty() {
        return Err("master-data region and revision are required".into());
    }
    MASTERDATA.with(|table| {
        let mut table = table.borrow_mut();
        table.next_handle = table.next_handle.wrapping_add(1).max(1);
        let handle = table.next_handle;
        let response = CreateResponse {
            handle,
            region: request.region.clone(),
            revision: request.revision.clone(),
            required_tables: PROFILE_MASTERDATA_TABLES,
        };
        table.sessions.insert(
            handle,
            MasterDataSession {
                data: JsonMasterData::new(&request.region),
                region: request.region,
                revision: request.revision,
                sealed: false,
            },
        );
        serde_json::to_string(&response).map_err(|error| error.to_string())
    })
}

pub fn put_table(handle: u32, input: &str) -> Result<String, String> {
    let request: PutTableRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse master-data table failed: {error}"))?;
    with_session_mut(handle, |session| {
        if session.sealed {
            return Err("master-data session is sealed".into());
        }
        session
            .data
            .insert_value(&request.name, request.table)
            .map_err(|error| error.to_string())?;
        serde_json::to_string(&serde_json::json!({
            "handle": handle,
            "loadedTables": session.data.loaded_tables().count(),
        }))
        .map_err(|error| error.to_string())
    })
}

pub fn seal(handle: u32) -> Result<String, String> {
    with_session_mut(handle, |session| {
        session.sealed = true;
        stats_value(handle, session)
    })
}

pub fn prepare(handle: u32, input: &str) -> Result<String, String> {
    let request: PrepareRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse profile preparation failed: {error}"))?;
    with_session(handle, |session| {
        if !session.sealed {
            return Err("master-data session must be sealed before profile preparation".into());
        }
        let profile = request.profile.as_ref().map(ProfileData::from_json);
        let prepared = prepare_profile(
            &request.card,
            profile.as_ref(),
            &session.data,
            &request.document_key,
            request.locale.as_deref().unwrap_or(&session.region),
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&prepared).map_err(|error| error.to_string())
    })
}

pub fn stats(handle: u32) -> Result<String, String> {
    with_session(handle, |session| stats_value(handle, session))
}

pub fn create_scene(handle: u32, input: &str) -> Result<String, String> {
    let request: CompileRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse profile compile request failed: {error}"))?;
    with_session(handle, |session| {
        if !session.sealed {
            return Err("master-data session must be sealed before scene creation".into());
        }
        let static_final = match request.frame_mode.as_deref().unwrap_or("animate") {
            "animate" => false,
            "final" => true,
            value => return Err(format!("unsupported frame mode {value}")),
        };
        let metrics = ResourceMetricMap(
            request
                .resource_metrics
                .into_iter()
                .map(|entry| {
                    (
                        (entry.namespace, entry.key),
                        ResourceMetric {
                            width: entry.width,
                            height: entry.height,
                        },
                    )
                })
                .collect(),
        );
        let profile = request.profile.as_ref().map(ProfileData::from_json);
        let mut line_indent = BTreeMap::new();
        for program in request.dynamic_programs {
            let source = LineIndentSource {
                percent: program.percent,
                advances_tmp: program.advances_tmp,
                rotation_deg: program.rotation_deg,
                scale_x: program.scale_x,
            };
            if line_indent
                .insert(program.layer_id.clone(), source)
                .is_some()
            {
                return Err(format!(
                    "duplicate dynamic program for layer {}",
                    program.layer_id
                ));
            }
        }
        let resolved = compile_profile_scene(
            &request.card,
            profile.as_ref(),
            &session.data,
            &request.document_key,
            request.locale.as_deref().unwrap_or(&session.region),
            &metrics,
            line_indent,
        )
        .map_err(|error| error.to_string())?;
        super::scene::create_compiled_profile(
            &request.document_key,
            &session.region,
            resolved,
            static_final,
        )
    })
}

pub fn destroy(handle: u32) -> bool {
    MASTERDATA.with(|table| table.borrow_mut().sessions.remove(&handle).is_some())
}

fn stats_value(handle: u32, session: &MasterDataSession) -> Result<String, String> {
    serde_json::to_string(&SessionStats {
        handle,
        region: &session.region,
        revision: &session.revision,
        sealed: session.sealed,
        tables: session.data.loaded_tables().collect(),
    })
    .map_err(|error| error.to_string())
}

fn with_session<T>(
    handle: u32,
    call: impl FnOnce(&MasterDataSession) -> Result<T, String>,
) -> Result<T, String> {
    MASTERDATA.with(|table| {
        let table = table.borrow();
        let session = table
            .sessions
            .get(&handle)
            .ok_or_else(|| format!("unknown master-data session {handle}"))?;
        call(session)
    })
}

fn with_session_mut<T>(
    handle: u32,
    call: impl FnOnce(&mut MasterDataSession) -> Result<T, String>,
) -> Result<T, String> {
    MASTERDATA.with(|table| {
        let mut table = table.borrow_mut();
        let session = table
            .sessions
            .get_mut(&handle)
            .ok_or_else(|| format!("unknown master-data session {handle}"))?;
        call(session)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn masterdata_session_reuses_tables_and_prepares_raw_authored_cards() {
        let created = super::create(r#"{"region":"cn","revision":"synthetic-v1"}"#).unwrap();
        assert!(created.contains("customProfileTextFonts"));
        let handle = serde_json::from_str::<serde_json::Value>(&created).unwrap()["handle"]
            .as_u64()
            .unwrap() as u32;
        super::put_table(handle, r#"{"name":"customProfileTextFonts","table":[{"id":1,"fontName":"FOT-RodinNTLGPro-DB"}]}"#).unwrap();
        super::put_table(handle, r##"{"name":"customProfileTextColors","table":[{"id":1,"colorCode":"#ffffff"},{"id":2,"colorCode":"#00000000"}]}"##).unwrap();
        super::seal(handle).unwrap();
        let card = serde_json::json!({"texts":[{"objectData":{"layer":1,"lock":false,"position":{"x":0.0,"y":0.0,"z":0.0},"rotation":{"w":1.0,"x":0.0,"y":0.0,"z":0.0},"scale":{"x":1.0,"y":1.0,"z":1.0},"visible":true},"colorId":1,"fontId":1,"lineSpacing":0.0,"outlineColorId":2,"outlineSize":0.0,"size":32.0,"text":"<line-indent=50%>42</line-indent>","type":0}]});
        let prepared = super::prepare(
            handle,
            &serde_json::json!({"documentKey":"session-card","card":card}).to_string(),
        )
        .unwrap();
        assert!(prepared.contains("FZLanTingHei-DB-GBK"));
        let prepared_value: serde_json::Value = serde_json::from_str(&prepared).unwrap();
        let source_key = prepared_value["layout_layers"][0]["dynamicLayerId"]
            .as_str()
            .unwrap();
        let response: serde_json::Value = serde_json::from_str(&super::create_scene(handle, &serde_json::json!({
            "documentKey": "session-card",
            "card": card,
            "frameMode": "final",
            "dynamicPrograms": [{ "layerId": source_key, "percent": 50.0, "advancesTmp": [24.0, 24.0], "rotationDeg": 0.0, "scaleX": 1.0 }]
        }).to_string()).unwrap()).unwrap();
        assert_eq!(
            response["snapshot"]["layer_sources"][0]["line_indent"]["percent"],
            50.0
        );
        assert!(response["snapshot"]["tick"].as_u64().unwrap() > 0);
        assert_ne!(
            response["snapshot"]["layer_commands"][0]["transform"],
            serde_json::json!({ "dx": 0.0, "dy": 0.0 })
        );
        assert!(super::super::scene::destroy(
            response["handle"].as_u64().unwrap() as u32
        ));
        let stats = super::stats(handle).unwrap();
        assert!(stats.contains("synthetic-v1"));
        assert!(super::destroy(handle));
    }
}
