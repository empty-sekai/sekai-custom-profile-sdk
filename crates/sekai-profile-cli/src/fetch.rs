//! 从 URL 前缀拉取 masterdata 表与素材。
//!
//! 三个 URL 各自是纯前缀，程序只在后面接 `/<table>.json` 或 `/<key>.png`，
//! 不插入 region / latest / assets 等任何约定子路径——兼容任意镜像布局。
//! 素材按内嵌静态清单分流：key 命中 `static_manifest` 走「静态」URL，否则
//! 走「动态」URL。不能按首段前缀划分——`honor/` 等前缀既含静态边框
//! （`honor/frame_degree_*`）又含动态图（`honor/<abn>/degree_*`）。
//!
//! HTTP 走同步 `ureq`，并发由标准库线程池承担，每个请求带指数退避重试。
//! 4xx（408 / 429 除外）表示对象不存在或不可读，不重试；5xx、408、429、
//! 连接失败与响应体读取中断都按退避重试。

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sekai_profile::assets::AssetStore;
use sekai_profile::codec::png::is_png;
use sekai_profile::masterdata::{OPTIONAL_TABLES, REQUIRED_TABLES};

use crate::static_manifest::is_static_key;

/// Path of an asset under an asset URL prefix.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum AssetUrlLayout {
    /// `<key>.png`
    #[default]
    Flat,
    /// The directories of the game's asset bundles.
    GameAssets,
}

impl AssetUrlLayout {
    fn relative_path(self, key: &str) -> String {
        match self {
            Self::Flat => format!("{key}.png"),
            Self::GameAssets => sekai_profile::asset_keys::key_to_s3_path(key, ""),
        }
    }
}

/// 单个请求的重试次数（首次 + 重试），指数退避。
const MAX_ATTEMPTS: u32 = 4;
/// 并发拉取素材的线程数。
const CONCURRENCY: usize = 8;

/// 去掉 URL 尾部斜杠，便于拼接。
fn trim_base(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// 一次 GET 的失败。
#[derive(Debug)]
enum FetchError {
    /// 服务端答复 4xx（408 / 429 除外）：对象不存在或不可读，未重试。
    Status(u16),
    /// 重试用尽仍失败（5xx、408、429、连接失败、响应体读取中断）。
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(code) => write!(f, "HTTP {code}"),
            Self::Failed(error) => f.write_str(error),
        }
    }
}

/// 4xx 表示对象不存在或不可读（缺素材是常态），不重试；408 Request Timeout
/// 与 429 Too Many Requests 是暂时性的，与 5xx 一样重试。
fn is_final_status(code: u16) -> bool {
    (400..500).contains(&code) && code != 408 && code != 429
}

/// 带指数退避重试的 GET，返回响应体字节。
fn get_bytes(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, FetchError> {
    let mut last_err = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        match agent.get(url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                match resp.into_reader().read_to_end(&mut buf) {
                    Ok(_) => return Ok(buf),
                    Err(e) => last_err = format!("reading the response body failed: {e}"),
                }
            }
            Err(ureq::Error::Status(code, _)) if is_final_status(code) => {
                return Err(FetchError::Status(code));
            }
            Err(ureq::Error::Status(code, _)) => last_err = format!("HTTP {code}"),
            Err(e) => last_err = e.to_string(),
        }
        // 最后一次失败后不再 sleep。
        if attempt + 1 < MAX_ATTEMPTS {
            let backoff_ms = 200u64 << attempt; // 200/400/800ms
            std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
        }
    }
    Err(FetchError::Failed(last_err))
}

/// 从 `masterdata_url` 逐表拉取，注入 provider，返回成功注入的表数。
///
/// 与 `from_dir` 同一口径：服务端对某表答复 4xx 视为该表不存在，记 warning
/// 跳过（[`OPTIONAL_TABLES`] 不记 warning）；表存在却拉不下来（重试用尽）、
/// 不是 UTF-8 或解析失败时整体报错，不带着残缺的 masterdata 继续渲染。
pub fn load_masterdata_url(
    provider: &mut sekai_profile::masterdata::JsonMasterDataProvider,
    masterdata_url: &str,
) -> Result<usize, String> {
    let base = trim_base(masterdata_url);
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let mut loaded = 0;
    for name in REQUIRED_TABLES.iter().chain(OPTIONAL_TABLES) {
        let url = format!("{base}/{name}.json");
        let bytes = match get_bytes(&agent, &url) {
            Ok(bytes) => bytes,
            Err(FetchError::Status(code)) if OPTIONAL_TABLES.contains(name) => {
                tracing::debug!(table = name, %url, "optional master data table is absent (HTTP {code})");
                continue;
            }
            Err(FetchError::Status(code)) => {
                tracing::warn!(table = name, %url, "master data table is missing (HTTP {code})");
                continue;
            }
            Err(e) => {
                return Err(format!(
                    "fetching master data table {name} ({url}) failed: {e}"
                ))
            }
        };
        let json = String::from_utf8(bytes)
            .map_err(|e| format!("master data table {name} ({url}) is not UTF-8: {e}"))?;
        provider.insert_table(name, &json)?;
        loaded += 1;
    }
    if loaded == 0 {
        return Err(format!("no master data table could be fetched from {base}"));
    }
    Ok(loaded)
}

/// 并发拉取缺失素材 key，注入 AssetStore。
/// `dynamic_url` 必填；`static_url` 可选（缺省时静态 key 也走 dynamic_url）。
/// 响应体不是 PNG（渲染器只解码 PNG）时计为失败、不注入。
/// 返回 (成功数, 失败数)。
pub fn load_assets_url(
    store: &Arc<AssetStore>,
    keys: &[String],
    dynamic_url: &str,
    static_url: Option<&str>,
    layout: AssetUrlLayout,
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
        let url = format!("{base}/{}", layout.relative_path(key));
        match get_bytes(&agent, &url) {
            Ok(bytes) if is_png(&bytes) => {
                store.put(key.clone(), bytes);
                ok.fetch_add(1, Ordering::Relaxed);
            }
            Ok(bytes) => {
                tracing::debug!(%key, %url, bytes = bytes.len(), "asset response is not a PNG");
                fail.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::debug!(%key, %url, "fetching the asset failed: {e}");
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

#[cfg(test)]
mod tests;
