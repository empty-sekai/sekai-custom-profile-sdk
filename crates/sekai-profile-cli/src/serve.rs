//! `serve`: render requests read as NDJSON from stdin.
//!
//! Each line is one JSON request. Requests are handled one at a time in the
//! order they arrive, and each gets one JSON response line on stdout. Logs go
//! to stderr.
//!
//! Requests:
//!   {"id": 1, "method": "render", "params": {...}}
//!   {"id": 2, "method": "reload_masterdata", "params": {"dir": "..."}}
//!   {"id": 3, "method": "ping"}
//!   {"id": 4, "method": "shutdown"}
//!
//! `render` params:
//!   profile: Profile API response object (optional)
//!   card: customProfileCard object or userCustomProfileCards array; defaults
//!         to the profile's userCustomProfileCards
//!   page: page number counted from 1 in the order the game shows them
//!         (default 1)
//!   seq: render the page with this seq instead of `page`
//!   format: "jpeg" or "png" (default "jpeg")
//!   transparent: keep the background transparent, PNG only (default false)
//!   quality: JPEG quality 1-100 (default 90)
//!   output: path to write the image to
//!   inline: return the image base64-encoded in `data` (default false)
//! At least one of `output` and `inline` is required. An absent or null
//! parameter takes its default; a parameter of the wrong type fails the
//! request.
//!
//! `render` response:
//!   {"id": 1, "ok": true, "result": {"path": "...", "bytes": 12345,
//!     "missing_assets": [...], "warnings": [...]}}
//! Failure:
//!   {"id": 1, "ok": false, "error": "..."}
//!
//! End of input is the same as `shutdown`.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::sync::Arc;

use base64::Engine;
use sekai_profile::assets::AssetStore;
use sekai_profile::masterdata::JsonMasterDataProvider;
use sekai_profile::region::Region;
use sekai_profile::{Background, CustomProfileRenderer, RenderConfig};
use serde_json::{json, Value};

use crate::{OutputFormat, PageSelection};

/// URL prefixes assets missing from the store are fetched from.
#[derive(Clone, Default)]
pub struct AssetUrls {
    pub dynamic: Option<String>,
    pub static_: Option<String>,
    pub layout: crate::fetch::AssetUrlLayout,
}

pub fn run(
    renderer: CustomProfileRenderer,
    assets: Arc<AssetStore>,
    asset_urls: AssetUrls,
    region: Region,
) -> ExitCode {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    tracing::info!("waiting for NDJSON requests on stdin");
    session(
        stdin.lock(),
        &mut stdout.lock(),
        &renderer,
        &assets,
        &asset_urls,
        region,
    )
}

/// Answers requests line by line until the input ends or `shutdown` arrives.
///
/// A line that is not UTF-8 or JSON, or a request with bad parameters, gets an
/// error response; the session goes on with the next line.
fn session(
    mut input: impl BufRead,
    output: &mut impl Write,
    renderer: &CustomProfileRenderer,
    assets: &Arc<AssetStore>,
    asset_urls: &AssetUrls,
    region: Region,
) -> ExitCode {
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match input.read_until(b'\n', &mut buffer) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                tracing::error!("reading stdin failed: {err}");
                break;
            }
        }
        let line = match std::str::from_utf8(&buffer) {
            Ok(line) => line.trim(),
            Err(err) => {
                write_response(
                    output,
                    &json!({"id": null, "ok": false, "error": format!("the request is not UTF-8: {err}")}),
                );
                continue;
            }
        };
        if line.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(err) => {
                write_response(
                    output,
                    &json!({"id": null, "ok": false, "error": format!("the request is not JSON: {err}")}),
                );
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");

        match method {
            "ping" => {
                write_response(output, &json!({"id": id, "ok": true, "result": "pong"}));
            }
            "shutdown" => {
                write_response(output, &json!({"id": id, "ok": true, "result": "bye"}));
                return ExitCode::SUCCESS;
            }
            "reload_masterdata" => {
                let result = handle_reload(renderer, &request, region);
                write_result(output, id, result);
            }
            "render" => {
                let result = handle_render(renderer, assets, asset_urls, &request);
                write_result(output, id, result);
            }
            other => {
                write_response(
                    output,
                    &json!({"id": id, "ok": false, "error": format!("unknown method {other}")}),
                );
            }
        }
    }

    tracing::info!("stdin closed");
    ExitCode::SUCCESS
}

fn write_result(output: &mut impl Write, id: Value, result: Result<Value, String>) {
    match result {
        Ok(result) => write_response(output, &json!({"id": id, "ok": true, "result": result})),
        Err(error) => write_response(output, &json!({"id": id, "ok": false, "error": error})),
    }
}

fn write_response(output: &mut impl Write, value: &Value) {
    if let Err(err) = serde_json::to_writer(&mut *output, value)
        .map_err(std::io::Error::other)
        .and_then(|_| output.write_all(b"\n"))
        .and_then(|_| output.flush())
    {
        tracing::error!("writing stdout failed: {err}");
    }
}

fn handle_reload(
    renderer: &CustomProfileRenderer,
    request: &Value,
    region: Region,
) -> Result<Value, String> {
    let dir = request
        .pointer("/params/dir")
        .and_then(|d| d.as_str())
        .ok_or("reload_masterdata needs params.dir")?;
    let provider = JsonMasterDataProvider::from_dir(std::path::Path::new(dir))?.with_region(region);
    let missing = provider.missing_tables();
    renderer.swap_masterdata(Arc::new(provider));
    Ok(json!({"reloaded": true, "missing_tables": missing}))
}

fn handle_render(
    renderer: &CustomProfileRenderer,
    assets: &Arc<AssetStore>,
    asset_urls: &AssetUrls,
    request: &Value,
) -> Result<Value, String> {
    let params = request.get("params").ok_or("render needs params")?;
    let RenderParams {
        card,
        profile,
        selection,
        format,
        transparent,
        quality,
        inline,
        output,
    } = render_params(params)?;
    if !inline && output.is_none() {
        return Err("render needs params.output or inline: true".into());
    }
    if transparent && format == OutputFormat::Jpeg {
        return Err("JPEG has no transparency; ask for a PNG".into());
    }

    let card = crate::select_card(card.cloned(), profile, selection)?;
    let profile = profile.map(sekai_profile::profile::ProfileData::from_json);
    let warnings = renderer.validate_card(&card);
    crate::fetch_assets(
        assets,
        asset_urls,
        &crate::missing_asset_keys(renderer, &card, profile.as_ref(), assets),
    );
    let missing_assets = crate::missing_asset_keys(renderer, &card, profile.as_ref(), assets);

    let config = RenderConfig {
        background: if transparent {
            Background::Transparent
        } else {
            Background::White
        },
        ..RenderConfig::default()
    };
    let data = crate::render_page(renderer, &card, profile.as_ref(), &config, format, quality)?;

    let mut result = json!({
        "bytes": data.len(),
        "missing_assets": missing_assets,
        "warnings": warnings,
    });
    if let Some(path) = output {
        std::fs::write(path, &data).map_err(|e| format!("write {path} failed: {e}"))?;
        result["path"] = json!(path);
    }
    if inline {
        result["data"] = json!(base64::engine::general_purpose::STANDARD.encode(&data));
    }
    Ok(result)
}

/// The parameters of a `render` request.
struct RenderParams<'a> {
    card: Option<&'a Value>,
    profile: Option<&'a Value>,
    selection: PageSelection,
    format: OutputFormat,
    transparent: bool,
    quality: u8,
    inline: bool,
    output: Option<&'a str>,
}

/// Reads the parameters of a `render` request. An absent or null parameter
/// takes its default; a parameter of the wrong type or out of range is an
/// error rather than a silent default.
fn render_params(params: &Value) -> Result<RenderParams<'_>, String> {
    let page = optional_param(params, "page", "an integer of at least 1", |value| {
        value
            .as_u64()
            .filter(|page| *page >= 1)
            .and_then(|page| usize::try_from(page - 1).ok())
    })?;
    let seq = optional_param(params, "seq", "an i32 integer", |value| {
        value.as_i64().and_then(|seq| i32::try_from(seq).ok())
    })?;
    let selection = match (page, seq) {
        (Some(_), Some(_)) => return Err("render takes params.page or params.seq, not both".into()),
        (_, Some(seq)) => PageSelection::Seq(seq),
        (index, None) => PageSelection::Index(index.unwrap_or(0)),
    };
    let format = optional_param(params, "format", "\"jpeg\" or \"png\"", |value| match value
        .as_str()?
    {
        "jpeg" | "jpg" => Some(OutputFormat::Jpeg),
        "png" => Some(OutputFormat::Png),
        _ => None,
    })?;
    Ok(RenderParams {
        card: optional_param(params, "card", "an object or an array", |value| {
            (value.is_object() || value.is_array()).then_some(value)
        })?,
        profile: optional_param(params, "profile", "an object", |value| {
            value.is_object().then_some(value)
        })?,
        selection,
        format: format.unwrap_or(OutputFormat::Jpeg),
        transparent: optional_param(params, "transparent", "a boolean", Value::as_bool)?
            .unwrap_or(false),
        quality: optional_param(params, "quality", "an integer from 1 to 100", |value| {
            value
                .as_u64()
                .filter(|quality| (1..=100).contains(quality))
                .map(|quality| quality as u8)
        })?
        .unwrap_or(90),
        inline: optional_param(params, "inline", "a boolean", Value::as_bool)?.unwrap_or(false),
        output: optional_param(params, "output", "a string", Value::as_str)?,
    })
}

fn optional_param<'a, T>(
    params: &'a Value,
    name: &str,
    expected: &str,
    read: impl FnOnce(&'a Value) -> Option<T>,
) -> Result<Option<T>, String> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => read(value)
            .map(Some)
            .ok_or_else(|| format!("render params.{name} must be {expected}, got {value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_session(input: &[u8]) -> Vec<Value> {
        let renderer = CustomProfileRenderer::new(Arc::new(JsonMasterDataProvider::empty()));
        let assets = Arc::new(AssetStore::new(1));
        let mut output = Vec::new();
        let _ = session(
            input,
            &mut output,
            &renderer,
            &assets,
            &AssetUrls::default(),
            Region::Cn,
        );
        String::from_utf8(output)
            .expect("responses are UTF-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("response is JSON"))
            .collect()
    }

    #[test]
    fn a_request_line_that_is_not_utf8_is_answered_and_the_session_continues() {
        let responses = run_session(b"\xff\xfe\n{\"id\": 1, \"method\": \"ping\"}\n");
        assert_eq!(responses.len(), 2, "{responses:?}");
        assert_eq!(responses[0]["id"], Value::Null);
        assert_eq!(responses[0]["ok"], false);
        assert_eq!(responses[1]["id"], 1);
        assert_eq!(responses[1]["result"], "pong");
    }

    #[test]
    fn malformed_requests_are_answered_without_ending_the_session() {
        let responses = run_session(
            b"not json\n[1, 2]\n{\"id\": 2, \"method\": \"render\"}\n{\"id\": 3, \"method\": \"ping\"}\n",
        );
        assert_eq!(responses.len(), 4, "{responses:?}");
        assert!(responses[..3]
            .iter()
            .all(|response| response["ok"] == false));
        assert_eq!(responses[2]["id"], 2);
        assert_eq!(responses[3]["result"], "pong");
    }

    #[test]
    fn render_params_accept_absent_and_null_values() {
        let absent = json!({ "page": null, "profile": null });
        let params = render_params(&absent).expect("params");
        assert_eq!(params.selection, PageSelection::Index(0));
        assert_eq!(params.format, OutputFormat::Jpeg);
        assert!(!params.transparent);
        assert_eq!(params.quality, 90);
        assert!(!params.inline);
        assert_eq!(params.output, None);
        assert!(params.profile.is_none());
        assert!(params.card.is_none());
        let given = json!({
            "page": 3, "format": "png", "transparent": true, "quality": 70,
            "inline": true, "output": "out.png", "profile": {}, "card": []
        });
        let params = render_params(&given).expect("params");
        assert_eq!(params.selection, PageSelection::Index(2));
        assert_eq!(params.format, OutputFormat::Png);
        assert!(params.transparent);
        assert_eq!(params.quality, 70);
        assert!(params.inline);
        assert_eq!(params.output, Some("out.png"));
        assert!(params.profile.is_some());
        assert!(params.card.is_some());
        let by_seq = json!({ "seq": -2 });
        let by_seq = render_params(&by_seq).expect("params");
        assert_eq!(by_seq.selection, PageSelection::Seq(-2));
    }

    #[test]
    fn render_params_of_the_wrong_type_or_range_are_rejected() {
        for params in [
            json!({ "page": 0 }),
            json!({ "page": "2" }),
            json!({ "page": 1.5 }),
            json!({ "seq": 4_294_967_297_i64 }),
            json!({ "page": 1, "seq": 1 }),
            json!({ "format": "webp" }),
            json!({ "format": 1 }),
            json!({ "transparent": "true" }),
            json!({ "quality": 0 }),
            json!({ "quality": 101 }),
            json!({ "inline": "true" }),
            json!({ "output": 7 }),
            json!({ "profile": "profile.json" }),
            json!({ "card": "card.json" }),
        ] {
            assert!(render_params(&params).is_err(), "{params} was accepted");
        }
    }
}
