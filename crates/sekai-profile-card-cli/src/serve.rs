//! `--serve` 常驻模式：stdin/stdout NDJSON 协议。
//!
//! 每行一个 JSON 请求，按到达顺序严格串行处理（匹配生产单 worker 模型）。
//! 响应写 stdout（每行一个 JSON），日志只走 stderr。
//!
//! 请求格式：
//!   {"id": 1, "method": "render", "params": {...}}
//!   {"id": 2, "method": "reload_masterdata", "params": {"dir": "..."}}
//!   {"id": 3, "method": "ping"}
//!   {"id": 4, "method": "shutdown"}
//!
//! `render` params：
//!   card: CustomProfileCard 或 UserCustomProfileCard 数组（必填）
//!   page: 数组时按 seq 选页（可选，i32 整数）
//!   profile: profile API 响应 JSON 对象（可选）
//!   format: jpeg|png|png-transparent（默认 jpeg）
//!   output: 输出文件路径（与 inline 二选一；都缺省时报错）
//!   inline: true 时响应 data 字段返 base64（默认 false）
//! 可选参数缺省或为 null 时取默认值；类型不符时该请求报错。
//!
//! `render` 响应：
//!   {"id": 1, "ok": true, "result": {"path": "...", "bytes": 12345,
//!     "missing_assets": [...], "warnings": [...]}}
//! 失败：
//!   {"id": 1, "ok": false, "error": "..."}
//!
//! stdin EOF 与 `shutdown` 同义。

use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::sync::Arc;

use base64::Engine;
use sekai_profile_renderer::assets::AssetStore;
use sekai_profile_renderer::region::Region;
use sekai_profile_renderer::renderer::CustomProfileRenderer;
use sekai_profile_renderer_host::JsonMasterDataProvider;
use serde_json::{json, Value};

/// 可选的素材 URL 前缀：缺失素材按需从此拉取（动态 + 静态）。
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
    tracing::info!("serve 模式就绪，等待 NDJSON 请求");
    session(
        stdin.lock(),
        &mut stdout.lock(),
        &renderer,
        &assets,
        &asset_urls,
        region,
    )
}

/// 逐行处理请求，直到输入结束或收到 `shutdown`。
///
/// 单行请求出错（非 UTF-8、非法 JSON、参数错误）只回报该请求的错误，
/// 会话继续处理后续请求。
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
                tracing::error!("读取 stdin 失败: {err}");
                break;
            }
        }
        let line = match std::str::from_utf8(&buffer) {
            Ok(line) => line.trim(),
            Err(err) => {
                write_response(
                    output,
                    &json!({"id": null, "ok": false, "error": format!("请求不是合法 UTF-8: {err}")}),
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
                    &json!({"id": null, "ok": false, "error": format!("请求不是合法 JSON: {err}")}),
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
                    &json!({"id": id, "ok": false, "error": format!("未知方法: {other}")}),
                );
            }
        }
    }

    tracing::info!("stdin 关闭，退出");
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
        tracing::error!("写 stdout 失败: {err}");
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
        .ok_or("reload_masterdata 缺少 params.dir")?;
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
    let params = request.get("params").ok_or("render 缺少 params")?;
    let card_value = params
        .get("card")
        .cloned()
        .ok_or("render 缺少 params.card")?;
    let RenderParams {
        page,
        format,
        inline,
        output,
        profile,
    } = render_params(params)?;
    if !inline && output.is_none() {
        return Err("render 需要 params.output（或 inline:true）".into());
    }

    let card = crate::card_from_value(card_value, page)?;
    let profile = profile.map(sekai_profile_renderer::profile::ProfileData::from_json);

    let warnings = renderer.validate_card(&card);

    // 配了 --assets-url 时，按需从 URL 补齐本地缺失的素材。
    if let Some(dyn_url) = &asset_urls.dynamic {
        let want =
            crate::missing_asset_keys_with_profile(renderer, &card, profile.as_ref(), assets);
        if !want.is_empty() {
            let (ok, fail) = crate::fetch::load_assets_url(
                assets,
                &want,
                dyn_url,
                asset_urls.static_.as_deref(),
                asset_urls.layout,
            );
            tracing::debug!(ok, fail, "serve 素材 URL 拉取完成");
        }
    }

    let missing_assets = crate::missing_asset_keys(renderer, &card, profile.as_ref(), assets);

    let data = crate::render_with_format(renderer, &card, profile.as_ref(), format)?;

    let mut result = json!({
        "bytes": data.len(),
        "missing_assets": missing_assets,
        "warnings": warnings,
    });
    if let Some(path) = output {
        std::fs::write(path, &data).map_err(|e| format!("写出 {path} 失败: {e}"))?;
        result["path"] = json!(path);
    }
    if inline {
        result["data"] = json!(base64::engine::general_purpose::STANDARD.encode(&data));
    }
    Ok(result)
}

/// `render` 的可选参数。
struct RenderParams<'a> {
    page: Option<i32>,
    format: &'a str,
    inline: bool,
    output: Option<&'a str>,
    profile: Option<&'a Value>,
}

/// 读取 `render` 的可选参数。缺省或 `null` 取默认值；给出但类型或范围不符时报错，
/// 不静默回落到默认值。
fn render_params(params: &Value) -> Result<RenderParams<'_>, String> {
    Ok(RenderParams {
        page: optional_param(params, "page", "i32 范围内的整数", |value| {
            value.as_i64().and_then(|page| i32::try_from(page).ok())
        })?,
        format: optional_param(params, "format", "字符串", Value::as_str)?.unwrap_or("jpeg"),
        inline: optional_param(params, "inline", "布尔值", Value::as_bool)?.unwrap_or(false),
        output: optional_param(params, "output", "字符串", Value::as_str)?,
        profile: optional_param(params, "profile", "对象", |value| {
            value.is_object().then_some(value)
        })?,
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
            .ok_or_else(|| format!("render params.{name} 必须是{expected}，收到 {value}")),
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
        assert_eq!(params.page, None);
        assert_eq!(params.format, "jpeg");
        assert!(!params.inline);
        assert_eq!(params.output, None);
        assert!(params.profile.is_none());
        let given = json!({
            "page": 3, "format": "png", "inline": true, "output": "out.png", "profile": {}
        });
        let params = render_params(&given).expect("params");
        assert_eq!(params.page, Some(3));
        assert_eq!(params.format, "png");
        assert!(params.inline);
        assert_eq!(params.output, Some("out.png"));
        assert!(params.profile.is_some());
    }

    #[test]
    fn render_params_of_the_wrong_type_or_range_are_rejected() {
        for params in [
            json!({ "page": 4_294_967_297_i64 }),
            json!({ "page": "2" }),
            json!({ "page": 1.5 }),
            json!({ "format": 1 }),
            json!({ "inline": "true" }),
            json!({ "output": 7 }),
            json!({ "profile": "profile.json" }),
        ] {
            assert!(render_params(&params).is_err(), "{params} was accepted");
        }
    }
}
