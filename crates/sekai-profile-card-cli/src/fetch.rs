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

use sekai_profile_renderer::assets::AssetStore;
use sekai_profile_renderer::codec::png::is_png;
use sekai_profile_renderer_host::{OPTIONAL_TABLES, REQUIRED_TABLES};

use crate::static_manifest::is_static_key;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AssetUrlLayout {
    #[default]
    Flat,
    GameAssets,
}

impl AssetUrlLayout {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "flat" => Ok(Self::Flat),
            "game-assets" => Ok(Self::GameAssets),
            other => Err(format!(
                "unknown --asset-url-layout {other}; expected flat or game-assets"
            )),
        }
    }

    fn relative_path(self, key: &str) -> String {
        match self {
            Self::Flat => format!("{key}.png"),
            Self::GameAssets => sekai_profile_renderer::asset_keys::key_to_s3_path(key, ""),
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
                    Err(e) => last_err = format!("读取响应体失败: {e}"),
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
    provider: &mut sekai_profile_renderer_host::JsonMasterDataProvider,
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
                tracing::debug!(table = name, %url, "可选 masterdata 表不存在（HTTP {code}），跳过");
                continue;
            }
            Err(FetchError::Status(code)) => {
                tracing::warn!(table = name, %url, "masterdata 表缺失（HTTP {code}），跳过");
                continue;
            }
            Err(e) => return Err(format!("拉取 masterdata 表 {name}（{url}）失败: {e}")),
        };
        let json = String::from_utf8(bytes)
            .map_err(|e| format!("masterdata 表 {name}（{url}）不是 UTF-8: {e}"))?;
        provider.insert_table(name, &json)?;
        loaded += 1;
    }
    if loaded == 0 {
        return Err(format!("从 {base} 未能拉取任何 masterdata 表"));
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
                tracing::debug!(%key, %url, bytes = bytes.len(), "素材响应不是 PNG，丢弃");
                fail.fetch_add(1, Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use sekai_profile_renderer::assets::AssetStore;
    use sekai_profile_renderer_host::JsonMasterDataProvider;

    use super::{load_assets_url, load_masterdata_url, AssetUrlLayout};

    /// Serves canned raw HTTP responses by request path. Each request takes
    /// the next queued response for its path and the last one repeats;
    /// unknown paths answer 404.
    fn serve(routes: Vec<(&str, Vec<Vec<u8>>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let base = format!("http://{}", listener.local_addr().expect("server address"));
        let routes = Mutex::new(
            routes
                .into_iter()
                .map(|(path, responses)| (path.to_string(), responses))
                .collect::<HashMap<_, _>>(),
        );
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let Ok(reader) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(reader);
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                loop {
                    let mut header = String::new();
                    match reader.read_line(&mut header) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if header == "\r\n" => break,
                        Ok(_) => {}
                    }
                }
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                let response = {
                    let mut routes = routes.lock().expect("routes");
                    match routes.get_mut(&path) {
                        Some(queue) if queue.len() > 1 => queue.remove(0),
                        Some(queue) => queue[0].clone(),
                        None => response(404, b"not found"),
                    }
                };
                let _ = stream.write_all(&response);
                let _ = stream.flush();
            }
        });
        base
    }

    fn response(status: u16, body: &[u8]) -> Vec<u8> {
        let mut bytes = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    fn png() -> Vec<u8> {
        sekai_profile_renderer::codec::png::encode_rgba(1, 1, &[1, 2, 3, 255]).expect("encode")
    }

    #[test]
    fn masterdata_tables_the_server_does_not_have_are_skipped() {
        let base = serve(vec![("/stamps.json", vec![response(200, b"[]")])]);
        let mut provider = JsonMasterDataProvider::empty();
        assert_eq!(load_masterdata_url(&mut provider, &base), Ok(1));
        assert!(provider.missing_tables().contains(&"cards"));
    }

    #[test]
    fn icon_tables_load_when_the_server_has_them() {
        use sekai_profile_renderer::masterdata::MasterDataProvider as _;

        let base = serve(vec![
            ("/stamps.json", vec![response(200, b"[]")]),
            (
                "/customProfileUserInterfaceIconResources.json",
                vec![response(
                    200,
                    br#"[{"id": 1, "customProfileResourceType": "user_interface_icon", "resourceLoadVal": "custom_profile/user_interface_icon", "fileName": "profile_icon_0001"}]"#,
                )],
            ),
        ]);
        let mut provider = JsonMasterDataProvider::empty();
        assert_eq!(load_masterdata_url(&mut provider, &base), Ok(2));
        assert_eq!(
            provider
                .resolve_resource("user_interface_icon", 1)
                .map(|info| info.asset_key())
                .as_deref(),
            Some("custom_profile/user_interface_icon/profile_icon_0001")
        );
        assert!(!provider
            .missing_tables()
            .contains(&"customProfileMaterialResources"));
    }

    #[test]
    fn a_masterdata_table_that_keeps_failing_fails_the_load() {
        let base = serve(vec![
            ("/stamps.json", vec![response(200, b"[]")]),
            ("/cards.json", vec![response(503, b"busy")]),
        ]);
        let mut provider = JsonMasterDataProvider::empty();
        let error = load_masterdata_url(&mut provider, &base).expect_err("cards did not load");
        assert!(error.contains("cards"), "{error}");
    }

    #[test]
    fn a_masterdata_table_that_does_not_parse_fails_the_load() {
        let base = serve(vec![
            ("/stamps.json", vec![response(200, b"[]")]),
            ("/honors.json", vec![response(200, b"<html></html>")]),
        ]);
        let mut provider = JsonMasterDataProvider::empty();
        let error = load_masterdata_url(&mut provider, &base).expect_err("honors did not parse");
        assert!(error.contains("honors"), "{error}");
    }

    #[test]
    fn transient_asset_failures_are_retried() {
        let image = png();
        let mut cut_short = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            image.len()
        )
        .into_bytes();
        cut_short.extend_from_slice(&image[..4]);
        let base = serve(vec![
            ("/cut.png", vec![cut_short, response(200, &image)]),
            (
                "/busy.png",
                vec![response(429, b"slow down"), response(200, &image)],
            ),
        ]);
        let store = Arc::new(AssetStore::new(1));
        let keys = ["cut".to_string(), "busy".to_string()];
        assert_eq!(
            load_assets_url(&store, &keys, &base, None, AssetUrlLayout::Flat),
            (2, 0)
        );
        assert_eq!(store.image_size("cut"), Some((1, 1)));
        assert_eq!(store.image_size("busy"), Some((1, 1)));
    }

    #[test]
    fn an_asset_response_that_is_not_a_png_is_a_failure() {
        let base = serve(vec![(
            "/soft404.png",
            vec![response(200, b"<html>not here</html>")],
        )]);
        let store = Arc::new(AssetStore::new(1));
        assert_eq!(
            load_assets_url(
                &store,
                &["soft404".to_string()],
                &base,
                None,
                AssetUrlLayout::Flat
            ),
            (0, 1)
        );
        assert!(!store.contains("soft404"));
    }

    #[test]
    fn flat_layout_keeps_generic_prefix_contract() {
        assert_eq!(
            AssetUrlLayout::Flat.relative_path("honor/example/degree_main"),
            "honor/example/degree_main.png"
        );
    }

    #[test]
    fn game_assets_layout_resolves_bonds_families() {
        assert_eq!(
            AssetUrlLayout::GameAssets.relative_path("bonds_honor/chr_sd_01_01"),
            "bonds_honor/character/chr_sd_01_01/chr_sd_01_01.png"
        );
        assert_eq!(
            AssetUrlLayout::GameAssets.relative_path("bonds_honor/word/honorname_0102_01_01"),
            "bonds_honor/word/honorname_0102_01_01/honorname_0102_01_01.png"
        );
    }
}
