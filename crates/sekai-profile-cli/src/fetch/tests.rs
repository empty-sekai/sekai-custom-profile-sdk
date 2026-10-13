use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use sekai_profile::assets::AssetStore;
use sekai_profile::masterdata::JsonMasterDataProvider;

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
    sekai_profile::codec::png::encode_rgba(1, 1, &[1, 2, 3, 255]).expect("encode")
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
    use sekai_profile::masterdata::MasterDataProvider as _;

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
