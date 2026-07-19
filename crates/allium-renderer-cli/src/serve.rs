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
//!   {"id": 5, "method": "render_honor", "params": {...}}
//!
//! `render` params：
//!   card: CustomProfileCard 或 UserCustomProfileCard 数组（必填）
//!   page: 数组时选页（可选）
//!   profile: profile API 响应 JSON（可选）
//!   format: jpeg|png|png-transparent（默认 jpeg）
//!   output: 输出文件路径（与 inline 二选一；都缺省时报错）
//!   inline: true 时响应 data 字段返 base64（默认 false）
//!
//! `render_honor` params：
//!   kind: normal|bonds（默认 normal）
//!   honorId / honorLevel / fullSize（必填；fullSize 默认 true）
//!   bonds 另接受 wordId / inverse / useUnitVirtualSinger
//!   quality: WebP quality 0..100（默认 90）
//!   output / inline：与 render 相同
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
use allium_renderer::region::Region;
use allium_renderer::renderer::CustomProfileRenderer;
use allium_renderer_host::JsonMasterDataProvider;
use base64::Engine;
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
                let result = handle_reload(&renderer, &request, region);
                write_result(&stdout, id, result);
            }
            "render" => {
                let result = handle_render(&renderer, &assets, &asset_urls, &request);
                write_result(&stdout, id, result);
            }
            "render_honor" => {
                let result = handle_render_honor(&renderer, &assets, &asset_urls, &request);
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

fn required_i32(params: &Value, name: &str) -> Result<i32, String> {
    let value = params
        .get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("render_honor missing integer params.{name}"))?;
    i32::try_from(value).map_err(|_| format!("render_honor params.{name} is out of i32 range"))
}

fn honor_card(
    params: &Value,
    kind: &str,
) -> Result<allium_renderer::types::CustomProfileCard, String> {
    let honor_id = required_i32(params, "honorId")?;
    let honor_level = params
        .get("honorLevel")
        .and_then(Value::as_i64)
        .unwrap_or(1);
    let full_size = params
        .get("fullSize")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let object_data = json!({
        "layer": 0,
        "lock": false,
        "position": {"x": 0.0, "y": 0.0, "z": 0.0},
        "rotation": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
        "scale": {"x": 1.0, "y": 1.0, "z": 1.0},
        "visible": true,
    });
    let card = match kind {
        "normal" => json!({"honors": [{
            "id": honor_id,
            "honorLevel": honor_level,
            "fullSize": full_size,
            "objectData": object_data,
        }]}),
        "bonds" => json!({"bondsHonors": [{
            "id": honor_id,
            "honorLevel": honor_level,
            "fullSize": full_size,
            "wordId": params.get("wordId").and_then(Value::as_i64).unwrap_or(0),
            "inverse": params.get("inverse").and_then(Value::as_bool).unwrap_or(false),
            "useUnitVirtualSinger": params.get("useUnitVirtualSinger").and_then(Value::as_bool).unwrap_or(false),
            "objectData": object_data,
        }]}),
        _ => return Err(format!("unsupported render_honor kind: {kind}")),
    };
    crate::card_from_value(card, None)
}

fn handle_render_honor(
    renderer: &CustomProfileRenderer,
    assets: &Arc<AssetStore>,
    asset_urls: &AssetUrls,
    request: &Value,
) -> Result<Value, String> {
    let params = request.get("params").ok_or("render_honor missing params")?;
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("normal");
    let honor_id = required_i32(params, "honorId")?;
    let honor_level = required_i32(params, "honorLevel")?;
    let full_size = params
        .get("fullSize")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let quality = params
        .get("quality")
        .and_then(Value::as_u64)
        .unwrap_or(90)
        .min(100) as u32;
    let inline = params
        .get("inline")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let output = params.get("output").and_then(Value::as_str);
    if !inline && output.is_none() {
        return Err("render_honor requires params.output or inline:true".into());
    }

    let card = honor_card(params, kind)?;
    if let Some(dynamic_url) = &asset_urls.dynamic {
        let wanted = crate::missing_asset_keys(renderer, &card, assets);
        if !wanted.is_empty() {
            let (ok, fail) = crate::fetch::load_assets_url(
                assets,
                &wanted,
                dynamic_url,
                asset_urls.static_.as_deref(),
                asset_urls.layout,
            );
            tracing::debug!(ok, fail, kind, honor_id, honor_level, "honor assets loaded");
        }
    }
    let missing_assets = crate::missing_asset_keys(renderer, &card, assets);
    if !missing_assets.is_empty() {
        return Err(format!(
            "render_honor missing {} asset(s): {}",
            missing_assets.len(),
            missing_assets.join(", ")
        ));
    }

    let artwork = match kind {
        "normal" => {
            renderer.render_static_honor_artwork(honor_id, honor_level, full_size, quality)?
        }
        "bonds" => renderer.render_bonds_honor_artwork(
            honor_id,
            honor_level,
            full_size,
            params.get("wordId").and_then(Value::as_i64).unwrap_or(0),
            params
                .get("inverse")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            params
                .get("useUnitVirtualSinger")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            quality,
        )?,
        _ => unreachable!("honor_card rejects unknown kinds"),
    };
    if let Some(path) = output {
        std::fs::write(path, &artwork.data)
            .map_err(|error| format!("write honor artwork {path} failed: {error}"))?;
    }
    let mut result = json!({
        "bytes": artwork.data.len(),
        "contentType": "image/webp",
        "height": artwork.height,
        "kind": kind,
        "missingAssets": [],
        "width": artwork.width,
    });
    if let Some(path) = output {
        result["path"] = json!(path);
    }
    if inline {
        result["data"] = json!(base64::engine::general_purpose::STANDARD.encode(&artwork.data));
    }
    Ok(result)
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
                asset_urls.layout,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_honor_card_preserves_level_and_size() {
        let params = json!({
            "honorId": 6030,
            "honorLevel": 7,
            "fullSize": false,
        });
        let card = honor_card(&params, "normal").expect("normal honor card");

        assert_eq!(card.honors.len(), 1);
        let honor = &card.honors[0];
        assert_eq!(honor.id, 6030);
        assert_eq!(honor.honor_level, 7);
        assert!(!honor.full_size);
        assert!(card.bonds_honors.is_empty());
    }

    #[test]
    fn bonds_honor_card_preserves_all_variant_fields() {
        let params = json!({
            "honorId": 1010201,
            "honorLevel": 10,
            "fullSize": false,
            "wordId": 1010202,
            "inverse": true,
            "useUnitVirtualSinger": true,
        });
        let card = honor_card(&params, "bonds").expect("bonds honor card");

        assert_eq!(card.bonds_honors.len(), 1);
        let honor = &card.bonds_honors[0];
        assert_eq!(honor.id, 1010201);
        assert_eq!(honor.honor_level, 10);
        assert_eq!(honor.word_id, 1010202);
        assert!(!honor.full_size);
        assert!(honor.inverse);
        assert!(honor.use_unit_virtual_singer);
        assert!(card.honors.is_empty());
    }

    #[test]
    fn honor_card_rejects_unknown_kind() {
        let error = honor_card(&json!({"honorId": 1}), "mystery")
            .expect_err("unknown honor kind must fail closed");
        assert!(error.contains("unsupported render_honor kind"));
    }

    #[test]
    fn required_integer_rejects_missing_and_out_of_range_values() {
        let missing = required_i32(&json!({}), "honorId").expect_err("missing integer must fail");
        assert!(missing.contains("missing integer params.honorId"));

        let out_of_range = required_i32(&json!({"honorId": i64::MAX}), "honorId")
            .expect_err("out-of-range integer must fail");
        assert!(out_of_range.contains("out of i32 range"));
    }
}
