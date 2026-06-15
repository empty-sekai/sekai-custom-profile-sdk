//! 从 URL 前缀拉取 masterdata 表与素材。
//!
//! 三个 URL 各自是纯前缀，程序只在后面接 `/<table>.json` 或 `/<key>.png`，
//! 不插入 region / latest / assets 等任何约定子路径——兼容任意镜像布局。
//! 素材按 key 第一段前缀分流到「静态」或「动态」URL（与 demo 同款划分）。
//!
//! HTTP 走同步 `ureq`，并发由标准库线程池承担，每个请求带指数退避重试。

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use allium_renderer::assets::AssetStore;
use allium_renderer_host::REQUIRED_TABLES;

/// 静态素材前缀：key 第一段命中则走 `--static-url`，否则走 `--assets-url`。
/// 与 demo 的 `STATIC_ASSET_PREFIXES` 保持一致。
const STATIC_ASSET_PREFIXES: &[&str] = &[
    "card",
    "chara_avatar",
    "general",
    "honor",
    "mysekai",
    "sprite",
    "ui",
];

/// 单个请求的重试次数（首次 + 重试），指数退避。
const MAX_ATTEMPTS: u32 = 4;
/// 并发拉取素材的线程数。
const CONCURRENCY: usize = 8;

fn is_static_key(key: &str) -> bool {
    let head = key.split('/').next().unwrap_or("");
    STATIC_ASSET_PREFIXES.contains(&head)
}

/// 去掉 URL 尾部斜杠，便于拼接。
fn trim_base(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// 带指数退避重试的 GET，返回响应体字节。
fn get_bytes(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let mut last_err = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        match agent.get(url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                return match resp.into_reader().read_to_end(&mut buf) {
                    Ok(_) => Ok(buf),
                    Err(e) => Err(format!("读取响应体失败: {e}")),
                };
            }
            Err(ureq::Error::Status(code, _)) => {
                // 4xx 不重试（缺素材是常态），5xx 重试。
                if (400..500).contains(&code) {
                    return Err(format!("HTTP {code}"));
                }
                last_err = format!("HTTP {code}");
            }
            Err(e) => last_err = e.to_string(),
        }
        // 最后一次失败后不再 sleep。
        if attempt + 1 < MAX_ATTEMPTS {
            let backoff_ms = 200u64 << attempt; // 200/400/800ms
            std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
        }
    }
    Err(last_err)
}

/// 从 `masterdata_url` 逐表拉取，注入 provider。
/// 返回成功注入的表数；缺表记 warning 跳过（与 from_dir 行为一致）。
pub fn load_masterdata_url(
    provider: &mut allium_renderer_host::JsonMasterDataProvider,
    masterdata_url: &str,
) -> Result<usize, String> {
    let base = trim_base(masterdata_url);
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let mut loaded = 0;
    for name in REQUIRED_TABLES {
        let url = format!("{base}/{name}.json");
        match get_bytes(&agent, &url) {
            Ok(bytes) => {
                let json = match String::from_utf8(bytes) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(table = name, "masterdata 表非 UTF-8，跳过: {e}");
                        continue;
                    }
                };
                match provider.insert_table(name, &json) {
                    Ok(()) => loaded += 1,
                    Err(e) => tracing::warn!(table = name, "masterdata 表解析失败，跳过: {e}"),
                }
            }
            Err(e) => tracing::warn!(table = name, %url, "masterdata 表拉取失败，跳过: {e}"),
        }
    }
    if loaded == 0 {
        return Err(format!("从 {base} 未能拉取任何 masterdata 表"));
    }
    Ok(loaded)
}

/// 并发拉取缺失素材 key，注入 AssetStore。
/// `dynamic_url` 必填；`static_url` 可选（缺省时静态 key 也走 dynamic_url）。
/// 返回 (成功数, 失败数)。
pub fn load_assets_url(
    store: &Arc<AssetStore>,
    keys: &[String],
    dynamic_url: &str,
    static_url: Option<&str>,
) -> (usize, usize) {
    if keys.is_empty() {
        return (0, 0);
    }
    let dynamic_base = trim_base(dynamic_url).to_string();
    let static_base = static_url.map(|u| trim_base(u).to_string());

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let next = AtomicUsize::new(0);
    let ok = AtomicUsize::new(0);
    let fail = AtomicUsize::new(0);

    let worker = || loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        if i >= keys.len() {
            break;
        }
        let key = &keys[i];
        let base = if is_static_key(key) {
            static_base.as_deref().unwrap_or(&dynamic_base)
        } else {
            &dynamic_base
        };
        let url = format!("{base}/{key}.png");
        match get_bytes(&agent, &url) {
            Ok(bytes) => {
                store.put(key.clone(), bytes);
                ok.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::debug!(%key, %url, "素材拉取失败: {e}");
                fail.fetch_add(1, Ordering::Relaxed);
            }
        }
    };

    let n = CONCURRENCY.min(keys.len());
    std::thread::scope(|scope| {
        for _ in 0..n {
            scope.spawn(worker);
        }
    });

    (ok.load(Ordering::Relaxed), fail.load(Ordering::Relaxed))
}
