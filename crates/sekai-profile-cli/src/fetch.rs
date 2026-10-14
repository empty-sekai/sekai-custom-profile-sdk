//! Fetching masterdata tables and assets from URL prefixes.
//!
//! Each of the three URLs is a plain prefix. The CLI appends only
//! `/<table>.json`, or `/` and the asset path of the chosen
//! [`AssetUrlLayout`], and inserts no conventional subpath such as a region,
//! `latest` or `assets`, so any mirror layout works. Assets are routed by the
//! embedded static manifest: a key listed in `static_manifest` goes to the
//! static URL, any other key to the dynamic URL. A first-segment prefix cannot
//! make that split, since prefixes such as `honor/` hold both static frames
//! (`honor/frame_degree_*`) and dynamic images (`honor/<abn>/degree_*`).
//!
//! HTTP uses blocking `ureq`; concurrency comes from scoped standard-library
//! threads, and every request retries with exponential backoff. A 4xx status
//! other than 408 and 429 means the object is absent or unreadable and is not
//! retried; 5xx, 408, 429, connection failures and interrupted body reads are
//! retried.

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

/// Attempts per request (the first try plus retries), with exponential
/// backoff.
const MAX_ATTEMPTS: u32 = 4;
/// Number of threads that fetch assets concurrently.
const CONCURRENCY: usize = 8;

/// Strips trailing slashes from a URL so paths can be appended.
fn trim_base(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// Failure of one GET.
#[derive(Debug)]
enum FetchError {
    /// The server answered 4xx (other than 408 and 429): the object is absent
    /// or unreadable. Not retried.
    Status(u16),
    /// Still failing after every retry (5xx, 408, 429, connection failure or
    /// an interrupted body read).
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

/// A 4xx status means the object is absent or unreadable (missing assets are
/// common) and is not retried. 408 Request Timeout and 429 Too Many Requests
/// are transient and are retried like 5xx.
fn is_final_status(code: u16) -> bool {
    (400..500).contains(&code) && code != 408 && code != 429
}

/// GET with exponential-backoff retries; returns the response body.
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
        // No sleep after the last failure.
        if attempt + 1 < MAX_ATTEMPTS {
            let backoff_ms = 200u64 << attempt; // 200/400/800ms
            std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
        }
    }
    Err(FetchError::Failed(last_err))
}

/// Fetches every table from `masterdata_url` into `provider` and returns the
/// number of tables loaded.
///
/// Follows the rules of `JsonMasterDataProvider::from_dir`: a 4xx answer for a
/// table means the table is absent and it is skipped with a warning (without
/// one for [`OPTIONAL_TABLES`]). A table that exists but cannot be fetched
/// after every retry, is not UTF-8 or does not parse fails the whole load, so
/// rendering never continues with incomplete masterdata.
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

/// Fetches the given asset keys concurrently into the [`AssetStore`].
/// `dynamic_url` is required; `static_url` is optional, and without it static
/// keys are fetched from `dynamic_url` too. A response that is not a PNG (the
/// renderer decodes only PNG) counts as a failure and is not stored.
/// Returns `(succeeded, failed)`.
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
