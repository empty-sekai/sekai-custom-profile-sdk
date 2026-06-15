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
//!   page: 数组时选页（可选）
//!   profile: profile API 响应 JSON（可选）
//!   format: jpeg|png|png-transparent（默认 jpeg）
//!   output: 输出文件路径（与 inline 二选一；都缺省时报错）
//!   inline: true 时响应 data 字段返 base64（默认 false）
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

use allium_renderer::assets::AssetStore;
use allium_renderer::renderer::CustomProfileRenderer;
use allium_renderer_host::JsonMasterDataProvider;
use base64::Engine;
use serde_json::{json, Value};

/// 可选的素材 URL 前缀：缺失素材按需从此拉取（动态 + 静态）。
#[derive(Clone, Default)]
pub struct AssetUrls {
    pub dynamic: Option<String>,
    pub static_: Option<String>,
}

pub fn run(
    renderer: CustomProfileRenderer,
    assets: Arc<AssetStore>,
    asset_urls: AssetUrls,
) -> ExitCode {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    tracing::info!("serve 模式就绪，等待 NDJSON 请求");

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                tracing::error!("读取 stdin 失败: {err}");
                break;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(err) => {
                write_response(
                    &stdout,
                    &json!({"id": null, "ok": false, "error": format!("请求不是合法 JSON: {err}")}),
                );
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");

        match method {
            "ping" => {
                write_response(&stdout, &json!({"id": id, "ok": true, "result": "pong"}));
            }
            "shutdown" => {
                write_response(&stdout, &json!({"id": id, "ok": true, "result": "bye"}));
                return ExitCode::SUCCESS;
            }
            "reload_masterdata" => {
                let result = handle_reload(&renderer, &request);
                write_result(&stdout, id, result);
            }
            "render" => {
                let result = handle_render(&renderer, &assets, &asset_urls, &request);
                write_result(&stdout, id, result);
            }
            other => {
                write_response(
                    &stdout,
                    &json!({"id": id, "ok": false, "error": format!("未知方法: {other}")}),
                );
            }
        }
    }

    tracing::info!("stdin 关闭，退出");
    ExitCode::SUCCESS
}

fn write_result(stdout: &std::io::Stdout, id: Value, result: Result<Value, String>) {
    match result {
        Ok(result) => write_response(stdout, &json!({"id": id, "ok": true, "result": result})),
        Err(error) => write_response(stdout, &json!({"id": id, "ok": false, "error": error})),
    }
}

fn write_response(stdout: &std::io::Stdout, value: &Value) {
    let mut lock = stdout.lock();
    if let Err(err) = serde_json::to_writer(&mut lock, value)
        .map_err(std::io::Error::other)
        .and_then(|_| lock.write_all(b"\n"))
        .and_then(|_| lock.flush())
    {
        tracing::error!("写 stdout 失败: {err}");
    }
}

fn handle_reload(renderer: &CustomProfileRenderer, request: &Value) -> Result<Value, String> {
    let dir = request
        .pointer("/params/dir")
        .and_then(|d| d.as_str())
        .ok_or("reload_masterdata 缺少 params.dir")?;
    let provider = JsonMasterDataProvider::from_dir(std::path::Path::new(dir))?;
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
    let page = params
        .get("page")
        .and_then(|p| p.as_i64())
        .map(|p| p as i32);
    let format = params
        .get("format")
        .and_then(|f| f.as_str())
        .unwrap_or("jpeg");
    let inline = params
        .get("inline")
        .and_then(|i| i.as_bool())
        .unwrap_or(false);
    let output = params.get("output").and_then(|o| o.as_str());
    if !inline && output.is_none() {
        return Err("render 需要 params.output（或 inline:true）".into());
    }

    let mut card = crate::card_from_value(card_value, page)?;
    let profile = params
        .get("profile")
        .filter(|p| !p.is_null())
        .map(|body| crate::enrich_from_profile_value(body, renderer, &mut card));

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
            );
            tracing::debug!(ok, fail, "serve 素材 URL 拉取完成");
        }
    }

    let missing_assets = crate::missing_asset_keys(renderer, &card, assets);

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
