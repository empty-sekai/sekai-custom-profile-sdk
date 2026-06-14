//! 浏览器 wasm 导出层（emscripten C ABI）。
//!
//! wasm-bindgen 不支持 `wasm32-unknown-emscripten`，互操作走 `extern "C"`：
//! JS 侧用 cwrap 包装，TS wrapper 负责类型与 Worker 调度。
//!
//! 导出函数约定：
//! - 字符串入参为 UTF-8 指针 + 长度；
//! - 二进制出参经 `alr_alloc`/`alr_free` 管理的线性内存传递；
//! - 返回 0 表示成功，非 0 为错误码，错误文本经 `alr_last_error` 取回。

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::{c_char, c_int};
use std::sync::Arc;

use allium_renderer::assets::AssetStore;
use allium_renderer::renderer::CustomProfileRenderer;
use allium_renderer_host::JsonMasterDataProvider;

// wasm（emscripten）单线程运行，全局可变状态用 thread_local 管理。
thread_local! {
    static STATE: RefCell<WasmState> = RefCell::new(WasmState::default());
}

#[derive(Default)]
struct WasmState {
    provider: Option<JsonMasterDataProvider>,
    renderer: Option<CustomProfileRenderer>,
    assets: Option<Arc<AssetStore>>,
    last_error: String,
    /// 待注入的表（renderer 构建前缓存）
    pending_tables: HashMap<String, String>,
}

fn set_error(message: impl Into<String>) -> c_int {
    STATE.with(|state| state.borrow_mut().last_error = message.into());
    1
}

/// 分配 wasm 线性内存（JS 侧写入入参用）。
#[no_mangle]
pub extern "C" fn alr_alloc(size: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(size);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// 释放 `alr_alloc` 或渲染输出的内存。
///
/// # Safety
/// `ptr` 必须来自 `alr_alloc(size)` 或本库返回的输出缓冲。
#[no_mangle]
pub unsafe extern "C" fn alr_free(ptr: *mut u8, size: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, size));
    }
}

/// 取最近一次错误文本。返回指针 + 写出长度（*len）。
///
/// # Safety
/// `len` 必须指向合法的 usize。返回的指针在下一次 API 调用前有效。
#[no_mangle]
pub unsafe extern "C" fn alr_last_error(len: *mut usize) -> *const c_char {
    STATE.with(|state| {
        let state = state.borrow();
        *len = state.last_error.len();
        state.last_error.as_ptr() as *const c_char
    })
}

unsafe fn slice_arg<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], String> {
    if ptr.is_null() {
        return Err("空指针入参".into());
    }
    Ok(std::slice::from_raw_parts(ptr, len))
}

unsafe fn str_arg<'a>(ptr: *const u8, len: usize) -> Result<&'a str, String> {
    std::str::from_utf8(slice_arg(ptr, len)?).map_err(|e| format!("入参不是合法 UTF-8: {e}"))
}

/// 注入一张 masterdata 表（JSON 字符串）。需在 `alr_init` 前调用。
///
/// # Safety
/// 指针/长度必须描述合法的 UTF-8 缓冲。
#[no_mangle]
pub unsafe extern "C" fn alr_load_masterdata(
    name_ptr: *const u8,
    name_len: usize,
    json_ptr: *const u8,
    json_len: usize,
) -> c_int {
    let (name, json) = match (str_arg(name_ptr, name_len), str_arg(json_ptr, json_len)) {
        (Ok(name), Ok(json)) => (name.to_string(), json.to_string()),
        (Err(e), _) | (_, Err(e)) => return set_error(e),
    };
    STATE.with(|state| {
        state.borrow_mut().pending_tables.insert(name, json);
    });
    0
}

/// 注册内存字体（family + 字体字节）。
///
/// # Safety
/// 指针/长度必须描述合法缓冲；family 必须是 UTF-8。
#[no_mangle]
pub unsafe extern "C" fn alr_register_font(
    family_ptr: *const u8,
    family_len: usize,
    bytes_ptr: *const u8,
    bytes_len: usize,
) -> c_int {
    #[cfg(feature = "skia")]
    {
        let family = match str_arg(family_ptr, family_len) {
            Ok(f) => f.to_string(),
            Err(e) => return set_error(e),
        };
        let bytes = match slice_arg(bytes_ptr, bytes_len) {
            Ok(b) => b.to_vec(),
            Err(e) => return set_error(e),
        };
        allium_renderer::sdf::outline::register_font_bytes(&family, bytes);
        0
    }
    #[cfg(not(feature = "skia"))]
    {
        let _ = (family_ptr, family_len, bytes_ptr, bytes_len);
        set_error("此构建未启用 skia")
    }
}

/// 用已注入的表初始化渲染器。重复调用会重建（等效热替换 masterdata）。
#[no_mangle]
pub extern "C" fn alr_init() -> c_int {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let mut provider = JsonMasterDataProvider::empty();
        let tables = std::mem::take(&mut state.pending_tables);
        for (name, json) in &tables {
            if let Err(e) = provider.insert_table(name, json) {
                state.pending_tables = tables.clone();
                state.last_error = e;
                return 1;
            }
        }
        let assets = state
            .assets
            .get_or_insert_with(|| Arc::new(AssetStore::new(128)))
            .clone();
        let missing = provider.missing_tables();
        if !missing.is_empty() {
            // 缺表不是致命错误：对应元素按缺映射渲染。记录供 JS 侧诊断。
            state.last_error = format!("缺失表: {missing:?}");
        }
        let shared = Arc::new(provider);
        match &state.renderer {
            Some(renderer) => renderer.swap_masterdata(shared),
            None => {
                state.renderer =
                    Some(CustomProfileRenderer::new(shared).with_assets(assets));
            }
        }
        state.provider = None;
        0
    })
}

/// 收集名片所需素材 key，返回 JSON 数组字符串。
///
/// 出参：`out_ptr`/`out_len` 写出缓冲指针与长度，调用方用 `alr_free` 释放。
///
/// # Safety
/// 所有指针必须合法；card JSON 必须是 UTF-8。
#[no_mangle]
pub unsafe extern "C" fn alr_collect_asset_keys(
    card_ptr: *const u8,
    card_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> c_int {
    let card_json = match str_arg(card_ptr, card_len) {
        Ok(j) => j,
        Err(e) => return set_error(e),
    };
    let card: allium_renderer::types::CustomProfileCard = match serde_json::from_str(card_json) {
        Ok(card) => card,
        Err(e) => return set_error(format!("解析名片失败: {e}")),
    };
    STATE.with(|state| {
        let state = state.borrow();
        let Some(renderer) = &state.renderer else {
            drop(state);
            return set_error("尚未调用 alr_init");
        };
        let md = renderer.snapshot_masterdata();
        let keys = allium_renderer::asset_keys::collect_card_asset_keys(&card, &md);
        let json = serde_json::to_vec(&keys).unwrap_or_else(|_| b"[]".to_vec());
        write_out(json, out_ptr, out_len);
        0
    })
}

/// 注入素材（key + 编码图片字节）。
///
/// # Safety
/// 指针/长度必须描述合法缓冲；key 必须是 UTF-8。
#[no_mangle]
pub unsafe extern "C" fn alr_put_asset(
    key_ptr: *const u8,
    key_len: usize,
    bytes_ptr: *const u8,
    bytes_len: usize,
) -> c_int {
    let key = match str_arg(key_ptr, key_len) {
        Ok(k) => k.to_string(),
        Err(e) => return set_error(e),
    };
    let bytes = match slice_arg(bytes_ptr, bytes_len) {
        Ok(b) => b.to_vec(),
        Err(e) => return set_error(e),
    };
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let assets = state
            .assets
            .get_or_insert_with(|| Arc::new(AssetStore::new(128)))
            .clone();
        assets.put(key, bytes);
        0
    })
}

/// 渲染名片。format：0=JPEG 1=PNG 2=PNG透明底。
///
/// 出参：`out_ptr`/`out_len` 写出编码图片字节，调用方用 `alr_free` 释放。
///
/// # Safety
/// 所有指针必须合法；card/profile JSON 必须是 UTF-8。profile 可传空指针。
#[no_mangle]
pub unsafe extern "C" fn alr_render(
    card_ptr: *const u8,
    card_len: usize,
    profile_ptr: *const u8,
    profile_len: usize,
    format: c_int,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> c_int {
    let card_json = match str_arg(card_ptr, card_len) {
        Ok(j) => j,
        Err(e) => return set_error(e),
    };
    let mut card: allium_renderer::types::CustomProfileCard = match serde_json::from_str(card_json)
    {
        Ok(card) => card,
        Err(e) => return set_error(format!("解析名片失败: {e}")),
    };
    let profile_body: Option<serde_json::Value> = if profile_ptr.is_null() || profile_len == 0 {
        None
    } else {
        match str_arg(profile_ptr, profile_len) {
            Ok(json) => match serde_json::from_str(json) {
                Ok(value) => Some(value),
                Err(e) => return set_error(format!("解析 profile 失败: {e}")),
            },
            Err(e) => return set_error(e),
        }
    };

    STATE.with(|state| {
        let state = state.borrow();
        let Some(renderer) = &state.renderer else {
            drop(state);
            return set_error("尚未调用 alr_init");
        };
        let profile = profile_body.as_ref().map(|body| {
            let profile = allium_renderer::profile::ProfileData::from_json(body);
            let (honor_levels, bonds_levels, char_ranks) =
                allium_renderer::profile::build_honor_maps(body);
            renderer.enrich_honor_levels(&mut card, &honor_levels, &bonds_levels, &char_ranks);
            profile
        });
        let result = match format {
            0 => renderer.render_page_with_profile(&card, profile.as_ref()),
            1 => renderer.render_page_png_with_profile(&card, profile.as_ref()),
            2 => renderer.render_page_png_transparent_with_profile(&card, profile.as_ref()),
            other => Err(format!("不支持的格式码: {other}")),
        };
        drop(state);
        match result {
            Ok(data) => {
                write_out(data, out_ptr, out_len);
                0
            }
            Err(e) => set_error(e),
        }
    })
}

/// 分层裁剪渲染：所有可见元素绘到透明画布 → 裁剪不透明包围盒 → WebP 编码。
///
/// 出参：`out_ptr`/`out_len` 写出 WebP 字节（`alr_free` 释放）；`out_rect`
/// 指向 4 个连续 u32，依次写入裁剪框 `x, y, width, height`（画布坐标系）。
///
/// # Safety
/// 所有指针必须合法；card/profile JSON 必须是 UTF-8；profile 可传空指针。
/// `out_rect` 必须指向至少 16 字节（4×u32）的可写缓冲。
#[no_mangle]
pub unsafe extern "C" fn alr_render_layer_cropped(
    card_ptr: *const u8,
    card_len: usize,
    profile_ptr: *const u8,
    profile_len: usize,
    quality: c_int,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
    out_rect: *mut u32,
) -> c_int {
    #[cfg(feature = "skia")]
    {
        let card_json = match str_arg(card_ptr, card_len) {
            Ok(j) => j,
            Err(e) => return set_error(e),
        };
        let mut card: allium_renderer::types::CustomProfileCard =
            match serde_json::from_str(card_json) {
                Ok(card) => card,
                Err(e) => return set_error(format!("解析名片失败: {e}")),
            };
        let profile_body: Option<serde_json::Value> = if profile_ptr.is_null() || profile_len == 0 {
            None
        } else {
            match str_arg(profile_ptr, profile_len) {
                Ok(json) => match serde_json::from_str(json) {
                    Ok(value) => Some(value),
                    Err(e) => return set_error(format!("解析 profile 失败: {e}")),
                },
                Err(e) => return set_error(e),
            }
        };

        STATE.with(|state| {
            let state = state.borrow();
            let Some(renderer) = &state.renderer else {
                drop(state);
                return set_error("尚未调用 alr_init");
            };
            let profile = profile_body.as_ref().map(|body| {
                let profile = allium_renderer::profile::ProfileData::from_json(body);
                let (honor_levels, bonds_levels, char_ranks) =
                    allium_renderer::profile::build_honor_maps(body);
                renderer.enrich_honor_levels(&mut card, &honor_levels, &bonds_levels, &char_ranks);
                profile
            });
            let result =
                renderer.render_element_layer_cropped(&card, profile.as_ref(), quality.max(0) as u32);
            drop(state);
            match result {
                Ok(layer) => {
                    *out_rect.add(0) = layer.x;
                    *out_rect.add(1) = layer.y;
                    *out_rect.add(2) = layer.width;
                    *out_rect.add(3) = layer.height;
                    write_out(layer.data, out_ptr, out_len);
                    0
                }
                Err(e) => set_error(e),
            }
        })
    }
    #[cfg(not(feature = "skia"))]
    {
        let _ = (
            card_ptr, card_len, profile_ptr, profile_len, quality, out_ptr, out_len, out_rect,
        );
        set_error("此构建未启用 skia")
    }
}

/// 批量分层裁剪渲染：所有元素按 layer 升序逐层渲染为裁剪 WebP。
///
/// 出参（一次 FFI 拿全部 N 层）：
///   - `out_meta_ptr/len`：UTF-8 JSON 数组，每项 `{z, type, original_visible, x, y, w, h,
///     byte_offset, byte_length, properties?}`。`properties` 仅 `include_properties=true`
///     时填充；不可见层的 `byte_length=0`。
///   - `out_blob_ptr/len`：所有可见层 WebP 字节首尾相接的一整块；按 meta 中
///     `byte_offset`/`byte_length` 切片。
///
/// 两块缓冲都用 `alr_free` 释放。
///
/// # Safety
/// 所有指针必须合法；card/profile JSON 必须是 UTF-8；profile 可传空指针。
#[no_mangle]
pub unsafe extern "C" fn alr_render_all_layers(
    card_ptr: *const u8,
    card_len: usize,
    profile_ptr: *const u8,
    profile_len: usize,
    quality: c_int,
    include_properties: c_int,
    out_meta_ptr: *mut *mut u8,
    out_meta_len: *mut usize,
    out_blob_ptr: *mut *mut u8,
    out_blob_len: *mut usize,
) -> c_int {
    #[cfg(feature = "skia")]
    {
        let card_json = match str_arg(card_ptr, card_len) {
            Ok(j) => j,
            Err(e) => return set_error(e),
        };
        let mut card: allium_renderer::types::CustomProfileCard =
            match serde_json::from_str(card_json) {
                Ok(card) => card,
                Err(e) => return set_error(format!("解析名片失败: {e}")),
            };
        let profile_body: Option<serde_json::Value> = if profile_ptr.is_null() || profile_len == 0 {
            None
        } else {
            match str_arg(profile_ptr, profile_len) {
                Ok(json) => match serde_json::from_str(json) {
                    Ok(value) => Some(value),
                    Err(e) => return set_error(format!("解析 profile 失败: {e}")),
                },
                Err(e) => return set_error(e),
            }
        };

        STATE.with(|state| {
            let state = state.borrow();
            let Some(renderer) = &state.renderer else {
                drop(state);
                return set_error("尚未调用 alr_init");
            };
            let profile = profile_body.as_ref().map(|body| {
                let profile = allium_renderer::profile::ProfileData::from_json(body);
                let (honor_levels, bonds_levels, char_ranks) =
                    allium_renderer::profile::build_honor_maps(body);
                renderer.enrich_honor_levels(&mut card, &honor_levels, &bonds_levels, &char_ranks);
                profile
            });
            let result = renderer.render_all_layers_cropped(
                &card,
                profile.as_ref(),
                quality.max(0) as u32,
                include_properties != 0,
            );
            drop(state);
            match result {
                Ok(layers) => {
                    // 拼接 blob + 构造 meta
                    let mut blob: Vec<u8> = Vec::new();
                    let mut meta_arr: Vec<serde_json::Value> = Vec::with_capacity(layers.len());
                    for layer in &layers {
                        let byte_offset = blob.len();
                        let byte_length = layer.data.len();
                        blob.extend_from_slice(&layer.data);
                        let mut entry = serde_json::json!({
                            "z": layer.z,
                            "type": layer.element_type,
                            "original_visible": layer.original_visible,
                            "x": layer.x,
                            "y": layer.y,
                            "width": layer.width,
                            "height": layer.height,
                            "byte_offset": byte_offset,
                            "byte_length": byte_length,
                        });
                        if let Some(props) = &layer.properties {
                            entry.as_object_mut().unwrap()
                                .insert("properties".into(), props.clone());
                        }
                        meta_arr.push(entry);
                    }
                    let meta_json = match serde_json::to_vec(&meta_arr) {
                        Ok(j) => j,
                        Err(e) => return set_error(format!("序列化 meta 失败: {e}")),
                    };
                    write_out(meta_json, out_meta_ptr, out_meta_len);
                    write_out(blob, out_blob_ptr, out_blob_len);
                    0
                }
                Err(e) => set_error(e),
            }
        })
    }
    #[cfg(not(feature = "skia"))]
    {
        let _ = (
            card_ptr, card_len, profile_ptr, profile_len, quality, include_properties,
            out_meta_ptr, out_meta_len, out_blob_ptr, out_blob_len,
        );
        set_error("此构建未启用 skia")
    }
}

unsafe fn write_out(data: Vec<u8>, out_ptr: *mut *mut u8, out_len: *mut usize) {
    let mut data = data;
    data.shrink_to_fit();
    let len = data.len();
    let ptr = data.as_mut_ptr();
    std::mem::forget(data);
    *out_ptr = ptr;
    *out_len = len;
}
