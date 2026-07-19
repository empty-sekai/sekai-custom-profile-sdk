mod atlas;
mod authoring_runtime;
mod edt;
mod geometry;
mod glyph_plan;
mod layout;
mod masterdata_runtime;
mod scene;

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::rc::Rc;
use std::slice;
use std::sync::Once;

use allium_renderer_core::sdf_geometry::{AnalyticDistanceField, Vec2};
use base64::Engine;
use freetype::{face::LoadFlag, Library, RenderMode};
use serde::Serialize;
use web_time::Instant;

use self::geometry::extract_segments;

const TMP_POINT_SIZE: f32 = 75.0;
const TMP_SPREAD: f32 = 6.0;
const FONT_ENGINE_FINGERPRINT: &str = match option_env!("ALLIUM_FONT_ENGINE_FINGERPRINT") {
    Some(value) => value,
    None => "freetype-unknown:dev",
};
static PANIC_HOOK: Once = Once::new();

#[no_mangle]
pub extern "C" fn sdf_layout_freetype_probe() -> i32 {
    install_panic_hook();
    match Library::init() {
        Ok(_) => 1,
        Err(_) => -1,
    }
}
#[no_mangle]
pub extern "C" fn sdf_layout_freetype_contract_json() -> *mut c_char {
    into_c_string(
        serde_json::to_string(&FreeTypeContract {
            font_engine_fingerprint: FONT_ENGINE_FINGERPRINT,
            freetype_version: "2.13.2",
            modules: &["truetype", "cff", "sfnt", "psaux", "psnames", "smooth"],
            load_contract: "analytic:NO_BITMAP|NO_HINTING;edt:NO_HINTING;metrics:26d6-v1",
        })
        .unwrap_or_else(|_| "{\"error\":\"contract serialization failed\"}".to_string()),
    )
}

#[no_mangle]
pub unsafe extern "C" fn sdf_layout_freetype_build_glyph_json(
    font_ptr: *const u8,
    font_len: usize,
    codepoints_ptr: *const u32,
    codepoints_len: usize,
    region_ptr: *const c_char,
    family_ptr: *const c_char,
    font_source_hash_ptr: *const c_char,
) -> *mut c_char {
    install_panic_hook();
    let result = build_glyph_batch_json(
        font_ptr,
        font_len,
        codepoints_ptr,
        codepoints_len,
        region_ptr,
        family_ptr,
        font_source_hash_ptr,
        0,
    );
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_layout_freetype_build_glyph_json_edt(
    font_ptr: *const u8,
    font_len: usize,
    codepoints_ptr: *const u32,
    codepoints_len: usize,
    region_ptr: *const c_char,
    family_ptr: *const c_char,
    font_source_hash_ptr: *const c_char,
    supersample: usize,
) -> *mut c_char {
    install_panic_hook();
    let result = build_glyph_batch_json(
        font_ptr,
        font_len,
        codepoints_ptr,
        codepoints_len,
        region_ptr,
        family_ptr,
        font_source_hash_ptr,
        supersample.clamp(1, 4),
    );
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_layout_freetype_map_glyphs_json(
    font_ptr: *const u8,
    font_len: usize,
    codepoints_ptr: *const u32,
    codepoints_len: usize,
    region_ptr: *const c_char,
    family_ptr: *const c_char,
    font_source_hash_ptr: *const c_char,
) -> *mut c_char {
    install_panic_hook();
    let result = map_glyphs_json(
        font_ptr,
        font_len,
        codepoints_ptr,
        codepoints_len,
        region_ptr,
        family_ptr,
        font_source_hash_ptr,
    );
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_layout_freetype_plan_glyphs_json(
    font_ptr: *const u8,
    font_len: usize,
    codepoints_ptr: *const u32,
    codepoints_len: usize,
    region_ptr: *const c_char,
    family_ptr: *const c_char,
    font_source_hash_ptr: *const c_char,
    backend_ptr: *const c_char,
    supersample: usize,
) -> *mut c_char {
    install_panic_hook();
    let result = (|| {
        if font_ptr.is_null() || codepoints_ptr.is_null() {
            return Err("null font or codepoint pointer".to_string());
        }
        glyph_plan::plan(
            slice::from_raw_parts(font_ptr, font_len).to_vec(),
            slice::from_raw_parts(codepoints_ptr, codepoints_len),
            read_c_string(region_ptr)?,
            read_c_string(family_ptr)?,
            read_c_string(font_source_hash_ptr)?,
            glyph_plan::RasterBackend::parse(&read_c_string(backend_ptr)?)?,
            supersample,
        )
        .and_then(|plan| serde_json::to_string(&plan).map_err(|error| error.to_string()))
    })();
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_layout_freetype_build_layout_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    install_panic_hook();
    let result = std::panic::catch_unwind(|| {
        read_utf8_slice(input_json_ptr, input_json_len).and_then(layout::build_layout_json)
    })
    .unwrap_or_else(|panic| {
        Err(if let Some(message) = panic.downcast_ref::<&str>() {
            format!("layout panic: {message}")
        } else if let Some(message) = panic.downcast_ref::<String>() {
            format!("layout panic: {message}")
        } else {
            "layout panic: unknown".to_string()
        })
    });
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_layout_freetype_glyph_demand_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    install_panic_hook();
    let result = std::panic::catch_unwind(|| {
        read_utf8_slice(input_json_ptr, input_json_len).and_then(layout::build_glyph_demand_json)
    })
    .unwrap_or_else(|_| Err("glyph-demand panic".to_string()));
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_atlas_create_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        let input = read_utf8_slice(input_json_ptr, input_json_len)?;
        let config: atlas::AtlasConfig =
            serde_json::from_str(input).map_err(|error| error.to_string())?;
        let (handle, stats) = atlas::create(config)?;
        serde_json::to_string(&serde_json::json!({ "handle": handle, "stats": stats }))
            .map_err(|error| error.to_string())
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_atlas_resolve_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        let input = read_utf8_slice(input_json_ptr, input_json_len)?;
        let request: atlas::AtlasResolveRequest =
            serde_json::from_str(input).map_err(|error| error.to_string())?;
        serde_json::to_string(&atlas::resolve(handle, request)?).map_err(|error| error.to_string())
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_atlas_pages_since_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        let input = read_utf8_slice(input_json_ptr, input_json_len)?;
        let request: atlas::AtlasPagesRequest =
            serde_json::from_str(input).map_err(|error| error.to_string())?;
        serde_json::to_string(&atlas::pages_since(handle, request)?)
            .map_err(|error| error.to_string())
    })
}

#[no_mangle]
pub extern "C" fn sdf_atlas_page_pixels_ptr(handle: u32, page: usize) -> *const u8 {
    atlas::page_pixels(handle, page)
        .map(|(pointer, _)| pointer)
        .unwrap_or(std::ptr::null())
}

#[no_mangle]
pub extern "C" fn sdf_atlas_page_pixels_len(handle: u32, page: usize) -> usize {
    atlas::page_pixels(handle, page)
        .map(|(_, length)| length)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn sdf_atlas_release(handle: u32, lease: u32) -> i32 {
    match atlas::release(handle, lease) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn sdf_atlas_destroy(handle: u32) -> i32 {
    if atlas::destroy(handle) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_scene_create_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| read_utf8_slice(input_json_ptr, input_json_len).and_then(scene::create))
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_create_blank_json() -> *mut c_char {
    core_json_call(authoring_runtime::create_blank)
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_import_profile_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len).and_then(authoring_runtime::import_profile)
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_restore_checkpoint_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(authoring_runtime::restore_checkpoint)
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_apply_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| authoring_runtime::apply(handle, input))
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_select_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| authoring_runtime::select(handle, input))
    })
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_elements_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::elements(handle))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_begin_gesture_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| authoring_runtime::begin_gesture(handle, input))
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_preview_gesture_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| authoring_runtime::preview_gesture(handle, input))
    })
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_commit_gesture_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::commit_gesture(handle))
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_cancel_gesture_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::cancel_gesture(handle))
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_append_page_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::append_page(handle))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_duplicate_page_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| authoring_runtime::duplicate_page(handle, input))
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_delete_page_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| authoring_runtime::delete_page(handle, input))
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_move_page_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| authoring_runtime::move_page(handle, input))
    })
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_undo_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::undo(handle))
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_redo_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::redo(handle))
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_export_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::export(handle))
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_checkpoint_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::checkpoint(handle))
}

#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_destroy(handle: u32) -> i32 {
    if authoring_runtime::destroy(handle) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_profile_scene_create_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len).and_then(scene::create_resolved_profile)
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_masterdata_create_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len).and_then(masterdata_runtime::create)
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_masterdata_put_table_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| masterdata_runtime::put_table(handle, input))
    })
}

#[no_mangle]
pub extern "C" fn sdf_renderer_core_masterdata_seal_json(handle: u32) -> *mut c_char {
    core_json_call(|| masterdata_runtime::seal(handle))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_profile_prepare_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| masterdata_runtime::prepare(handle, input))
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_profile_create_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| masterdata_runtime::create_scene(handle, input))
    })
}

#[no_mangle]
pub extern "C" fn sdf_renderer_core_masterdata_stats_json(handle: u32) -> *mut c_char {
    core_json_call(|| masterdata_runtime::stats(handle))
}

#[no_mangle]
pub extern "C" fn sdf_renderer_core_masterdata_destroy(handle: u32) -> i32 {
    if masterdata_runtime::destroy(handle) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn sdf_renderer_core_scene_advance_json(handle: u32, tick: u32) -> *mut c_char {
    core_json_call(|| scene::advance(handle, tick as u64))
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_scene_advance_binary(
    handle: u32,
    tick: u32,
    output: *mut u8,
    capacity: usize,
) -> usize {
    unsafe { scene::advance_binary(handle, tick as u64, output, capacity) }.unwrap_or_default()
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_scene_set_mask_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| scene::set_mask(handle, input))
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_scene_set_masks_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| scene::set_masks(handle, input))
    })
}
#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_scene_set_tab_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| scene::set_tab(handle, input))
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_scene_scroll_json(
    handle: u32,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| scene::scroll(handle, input))
    })
}

#[no_mangle]
pub extern "C" fn sdf_renderer_core_scene_dump_json(handle: u32) -> *mut c_char {
    core_json_call(|| scene::dump(handle))
}

#[no_mangle]
pub extern "C" fn sdf_renderer_core_scene_destroy(handle: u32) -> i32 {
    if scene::destroy(handle) {
        1
    } else {
        0
    }
}

#[derive(serde::Deserialize)]
struct LocaleResolveRequest {
    region: String,
    key: String,
}

#[derive(serde::Serialize)]
struct LocaleResolveResponse {
    region: String,
    key: String,
    value: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileResolveRequest {
    document_key: String,
    card: allium_renderer_core::profile_source::CustomProfileCard,
    snapshot: allium_renderer_core::profile_scene::ProfileResolveSnapshot,
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_resolve_locale_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        let input = read_utf8_slice(input_json_ptr, input_json_len)?;
        let request: LocaleResolveRequest =
            serde_json::from_str(input).map_err(|error| error.to_string())?;
        serde_json::to_string(&LocaleResolveResponse {
            value: allium_renderer_core::locale::resolve(&request.region, &request.key),
            region: request.region,
            key: request.key,
        })
        .map_err(|error| error.to_string())
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_resolve_profile_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        let input = read_utf8_slice(input_json_ptr, input_json_len)?;
        let request: ProfileResolveRequest =
            serde_json::from_str(input).map_err(|error| error.to_string())?;
        let resolved = allium_renderer_core::profile_scene::resolve_profile_scene(
            &request.card,
            &request.document_key,
            &request.snapshot,
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&resolved).map_err(|error| error.to_string())
    })
}

#[no_mangle]
pub unsafe extern "C" fn sdf_layout_freetype_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("[sdf-freetype panic] {info}");
        }));
    });
}

fn core_json_call<F>(call: F) -> *mut c_char
where
    F: FnOnce() -> Result<String, String>,
{
    install_panic_hook();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(call))
        .unwrap_or_else(|_| Err("renderer core panic".to_string()));
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

unsafe fn build_glyph_batch_json(
    font_ptr: *const u8,
    font_len: usize,
    codepoints_ptr: *const u32,
    codepoints_len: usize,
    region_ptr: *const c_char,
    family_ptr: *const c_char,
    font_source_hash_ptr: *const c_char,
    supersample: usize,
) -> Result<String, String> {
    if font_ptr.is_null() || codepoints_ptr.is_null() {
        return Err("null font or codepoint pointer".to_string());
    }
    let t0 = Instant::now();
    let font_bytes = slice::from_raw_parts(font_ptr, font_len).to_vec();
    let codepoints = slice::from_raw_parts(codepoints_ptr, codepoints_len);
    let region = read_c_string(region_ptr)?;
    let family = read_c_string(family_ptr)?;
    let font_source_hash = read_c_string(font_source_hash_ptr)?;

    let library = Library::init().map_err(|err| format!("FreeType init failed: {err:?}"))?;
    let t1 = Instant::now();
    let face = library
        .new_memory_face(Rc::new(font_bytes), 0)
        .map_err(|err| format!("load memory face failed: {err:?}"))?;
    face.set_char_size((TMP_POINT_SIZE as isize) * 64, 0, 72, 72)
        .map_err(|err| format!("set char size failed: {err:?}"))?;
    let t2 = Instant::now();

    let mut glyphs = Vec::new();
    let mut missing = Vec::new();
    let mut glyph_total_ms = 0.0f64;
    let mut total_pixel_count: usize = 0;
    for codepoint in codepoints {
        let Some(ch) = char::from_u32(*codepoint) else {
            missing.push(format!("U+{codepoint:04X}"));
            continue;
        };
        if ch == '\n' || ch == '\r' {
            continue;
        }
        let g0 = Instant::now();
        let glyph_result = if supersample > 0 {
            build_glyph_edt(&face, &region, &family, &font_source_hash, ch, supersample)
                .or_else(|_| build_glyph(&face, &region, &family, &font_source_hash, ch))
        } else {
            build_glyph(&face, &region, &family, &font_source_hash, ch)
        };
        match glyph_result {
            Ok(glyph) => {
                total_pixel_count += glyph.width * glyph.height;
                glyphs.push(glyph);
            }
            Err(message) => missing.push(format!("{family}:{ch}:{message}")),
        }
        glyph_total_ms += g0.elapsed().as_secs_f64() * 1000.0;
    }
    let t3 = Instant::now();

    let glyph_count = glyphs.len();
    serde_json::to_string(&GlyphBatch {
        region,
        family,
        font_source_hash,
        base_size: TMP_POINT_SIZE,
        spread: TMP_SPREAD,
        glyphs,
        missing,
        perf: GlyphBatchPerf {
            total_ms: duration_ms(t0, t3),
            face_load_ms: duration_ms(t1, t2),
            glyph_total_ms,
            glyph_count,
            per_glyph_avg_ms: if glyph_count > 0 {
                glyph_total_ms / glyph_count as f64
            } else {
                0.0
            },
            total_pixel_count,
            avg_pixels_per_glyph: if glyph_count > 0 {
                total_pixel_count as f64 / glyph_count as f64
            } else {
                0.0
            },
        },
    })
    .map_err(|err| format!("serialize glyph batch failed: {err}"))
}

unsafe fn map_glyphs_json(
    font_ptr: *const u8,
    font_len: usize,
    codepoints_ptr: *const u32,
    codepoints_len: usize,
    region_ptr: *const c_char,
    family_ptr: *const c_char,
    font_source_hash_ptr: *const c_char,
) -> Result<String, String> {
    if font_ptr.is_null() || codepoints_ptr.is_null() {
        return Err("null font or codepoint pointer".to_string());
    }
    let font_bytes = slice::from_raw_parts(font_ptr, font_len).to_vec();
    let codepoints = slice::from_raw_parts(codepoints_ptr, codepoints_len);
    let region = read_c_string(region_ptr)?;
    let family = read_c_string(family_ptr)?;
    let font_source_hash = read_c_string(font_source_hash_ptr)?;
    let library = Library::init().map_err(|err| format!("FreeType init failed: {err:?}"))?;
    let face = library
        .new_memory_face(Rc::new(font_bytes), 0)
        .map_err(|err| format!("load memory face failed: {err:?}"))?;
    let mut glyphs = Vec::with_capacity(codepoints.len());
    let mut missing = Vec::new();
    for codepoint in codepoints {
        let Some(ch) = char::from_u32(*codepoint) else {
            missing.push(format!("U+{codepoint:04X}"));
            continue;
        };
        match face.get_char_index(ch as usize) {
            Some(glyph_index) => glyphs.push(GlyphMapEntry {
                ch: ch.to_string(),
                glyph_index,
            }),
            None => missing.push(format!("U+{codepoint:04X}")),
        }
    }
    serde_json::to_string(&GlyphMapBatch {
        region,
        family,
        font_source_hash,
        glyphs,
        missing,
    })
    .map_err(|err| format!("serialize glyph map failed: {err}"))
}

fn duration_ms(start: Instant, end: Instant) -> f64 {
    (end - start).as_secs_f64() * 1000.0
}

unsafe fn read_c_string(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Ok(String::new());
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map(str::to_string)
        .map_err(|err| format!("invalid utf8 string: {err}"))
}

unsafe fn read_utf8_slice<'a>(ptr: *const u8, len: usize) -> Result<&'a str, String> {
    if ptr.is_null() {
        return Err("null utf8 pointer".to_string());
    }
    let bytes = slice::from_raw_parts(ptr, len);
    std::str::from_utf8(bytes).map_err(|err| format!("invalid utf8 slice: {err}"))
}

fn into_c_string(value: String) -> *mut c_char {
    CString::new(value)
        .unwrap_or_else(|_| CString::new("{\"error\":\"interior nul byte\"}").unwrap())
        .into_raw()
}

fn build_glyph(
    face: &freetype::Face,
    region: &str,
    family: &str,
    font_source_hash: &str,
    ch: char,
) -> Result<GlyphSdf, String> {
    let glyph_id = face
        .get_char_index(ch as usize)
        .ok_or_else(|| "missing cmap entry".to_string())?;
    face.load_glyph(glyph_id, LoadFlag::NO_BITMAP | LoadFlag::NO_HINTING)
        .map_err(|err| format!("load glyph failed: {err:?}"))?;

    let glyph = face.glyph();
    let metrics = glyph.metrics();
    let bear_x = metrics.horiBearingX as f32 / 64.0;
    let bear_y = metrics.horiBearingY as f32 / 64.0;
    let met_w = metrics.width as f32 / 64.0;
    let met_h = metrics.height as f32 / 64.0;
    let advance = metrics.horiAdvance as f32 / 64.0;

    let outline = &glyph.raw().outline;
    if outline.n_contours <= 0 || outline.n_points <= 0 {
        return Ok(empty_metric_glyph(
            region,
            family,
            font_source_hash,
            ch,
            glyph_id,
            advance,
        ));
    }

    let contours = unsafe { extract_segments(outline) };
    if contours.is_empty() {
        return Ok(empty_metric_glyph(
            region,
            family,
            font_source_hash,
            ch,
            glyph_id,
            advance,
        ));
    }

    let rect_left_px = bear_x.floor();
    let rect_top_px = bear_y.ceil();
    let rect_right_px = (bear_x + met_w).ceil();
    let rect_bottom_px = (bear_y - met_h).floor();
    let spread_px = TMP_SPREAD.ceil();
    let sample_left_px = rect_left_px - spread_px;
    let sample_top_px = rect_top_px + spread_px;
    let sample_right_px = rect_right_px + spread_px;
    let sample_bottom_px = rect_bottom_px - spread_px;

    let width = (sample_right_px - sample_left_px).max(1.0) as usize;
    let height = (sample_top_px - sample_bottom_px).max(1.0) as usize;
    let rect_left_26_6 = sample_left_px * 64.0;
    let rect_top_26_6 = sample_top_px * 64.0;

    let distance_field = AnalyticDistanceField::new(&contours);
    let mut pixels = vec![0u8; width * height];
    for py in 0..height {
        for px in 0..width {
            let point = Vec2::new(
                rect_left_26_6 + (px as f32 + 0.5) * 64.0,
                rect_top_26_6 - (py as f32 + 0.5) * 64.0,
            );
            let signed_distance_px = distance_field.signed_distance(point) / 64.0;
            let gray = (0.5 - signed_distance_px / (2.0 * TMP_SPREAD)).clamp(0.0, 1.0);
            pixels[py * width + px] = (gray * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    Ok(GlyphSdf {
        key: glyph_key(region, family, font_source_hash, ch),
        region: region.to_string(),
        family: family.to_string(),
        font_source_hash: font_source_hash.to_string(),
        ch: ch.to_string(),
        glyph_index: glyph_id,
        width,
        height,
        bearing_x: sample_left_px,
        bearing_y: sample_top_px,
        x_offset: sample_left_px,
        y_offset: -sample_top_px,
        advance,
        plane_bearing_x: bear_x,
        plane_bearing_y: bear_y,
        plane_width: met_w.max(1.0 / 64.0),
        plane_height: met_h.max(1.0 / 64.0),
        drawable: true,
        pixels_base64: encode_pixels(&pixels),
    })
}

fn build_glyph_edt(
    face: &freetype::Face,
    region: &str,
    family: &str,
    font_source_hash: &str,
    ch: char,
    supersample: usize,
) -> Result<GlyphSdf, String> {
    let glyph_id = face
        .get_char_index(ch as usize)
        .ok_or_else(|| "missing cmap entry".to_string())?;
    face.load_glyph(glyph_id, LoadFlag::NO_HINTING)
        .map_err(|err| format!("load glyph failed: {err:?}"))?;

    let glyph = face.glyph();
    let metrics = glyph.metrics();
    let bear_x = metrics.horiBearingX as f32 / 64.0;
    let bear_y = metrics.horiBearingY as f32 / 64.0;
    let met_w = metrics.width as f32 / 64.0;
    let met_h = metrics.height as f32 / 64.0;
    let advance = metrics.horiAdvance as f32 / 64.0;

    let outline = &glyph.raw().outline;
    if outline.n_contours <= 0 || outline.n_points <= 0 {
        return Ok(empty_metric_glyph(
            region,
            family,
            font_source_hash,
            ch,
            glyph_id,
            advance,
        ));
    }

    let rect_left_px = bear_x.floor();
    let rect_top_px = bear_y.ceil();
    let rect_right_px = (bear_x + met_w).ceil();
    let rect_bottom_px = (bear_y - met_h).floor();
    let spread_px = TMP_SPREAD.ceil();
    let sample_left_px = rect_left_px - spread_px;
    let sample_top_px = rect_top_px + spread_px;
    let sample_right_px = rect_right_px + spread_px;
    let sample_bottom_px = rect_bottom_px - spread_px;

    let width = (sample_right_px - sample_left_px).max(1.0) as usize;
    let height = (sample_top_px - sample_bottom_px).max(1.0) as usize;

    let ss = supersample.max(1);
    let raster_w = width * ss;
    let raster_h = height * ss;
    glyph
        .render_glyph(RenderMode::Normal)
        .map_err(|err| format!("render glyph failed: {err:?}"))?;
    let bitmap = glyph.bitmap();
    let bm_w = bitmap.width() as usize;
    let bm_h = bitmap.rows() as usize;
    let bm_left = glyph.bitmap_left();
    let bm_top = glyph.bitmap_top();

    let mut inside = vec![false; raster_w * raster_h];
    if bm_w > 0 && bm_h > 0 {
        let buffer = bitmap.buffer();
        let pitch = bitmap.pitch().unsigned_abs() as usize;
        for ry in 0..raster_h {
            for rx in 0..raster_w {
                let px_26_6 = (sample_left_px + (rx as f32 + 0.5) / ss as f32) * 64.0;
                let py_26_6 = (sample_top_px - (ry as f32 + 0.5) / ss as f32) * 64.0;
                let bx = ((px_26_6 / 64.0) - bm_left as f32).floor() as isize;
                let by = (bm_top as f32 - (py_26_6 / 64.0)).floor() as isize;
                if bx >= 0 && by >= 0 && (bx as usize) < bm_w && (by as usize) < bm_h {
                    let coverage = buffer[by as usize * pitch + bx as usize];
                    inside[ry * raster_w + rx] = coverage >= 128;
                }
            }
        }
    }

    let sd_ss = edt::signed_distance_from_mask(&inside, raster_w, raster_h);

    let mut pixels = vec![0u8; width * height];
    for py in 0..height {
        for px in 0..width {
            let mut sum = 0.0f32;
            for sy in 0..ss {
                for sx in 0..ss {
                    let idx = (py * ss + sy) * raster_w + (px * ss + sx);
                    sum += sd_ss[idx];
                }
            }
            let dist_px = sum / (ss * ss) as f32 / ss as f32;
            let gray = (0.5 - dist_px / (2.0 * TMP_SPREAD)).clamp(0.0, 1.0);
            pixels[py * width + px] = (gray * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    Ok(GlyphSdf {
        key: glyph_key(region, family, font_source_hash, ch),
        region: region.to_string(),
        family: family.to_string(),
        font_source_hash: font_source_hash.to_string(),
        ch: ch.to_string(),
        glyph_index: glyph_id,
        width,
        height,
        bearing_x: sample_left_px,
        bearing_y: sample_top_px,
        x_offset: sample_left_px,
        y_offset: -sample_top_px,
        advance,
        plane_bearing_x: bear_x,
        plane_bearing_y: bear_y,
        plane_width: met_w.max(1.0 / 64.0),
        plane_height: met_h.max(1.0 / 64.0),
        drawable: true,
        pixels_base64: encode_pixels(&pixels),
    })
}
fn empty_metric_glyph(
    region: &str,
    family: &str,
    font_source_hash: &str,
    ch: char,
    glyph_index: u32,
    advance: f32,
) -> GlyphSdf {
    GlyphSdf {
        key: glyph_key(region, family, font_source_hash, ch),
        region: region.to_string(),
        family: family.to_string(),
        font_source_hash: font_source_hash.to_string(),
        ch: ch.to_string(),
        glyph_index,
        width: 1,
        height: 1,
        bearing_x: 0.0,
        bearing_y: 0.0,
        x_offset: 0.0,
        y_offset: 0.0,
        advance,
        plane_bearing_x: 0.0,
        plane_bearing_y: 0.0,
        plane_width: advance.max(1.0 / 64.0),
        plane_height: 0.0,
        drawable: false,
        pixels_base64: encode_pixels(&[0]),
    }
}

fn encode_pixels(pixels: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(pixels)
}

fn glyph_key(region: &str, family: &str, font_source_hash: &str, ch: char) -> String {
    format!("{region}\u{0}{font_source_hash}\u{0}{family}\u{0}{ch}")
}

#[derive(Serialize)]
struct GlyphBatch {
    region: String,
    family: String,
    font_source_hash: String,
    base_size: f32,
    spread: f32,
    glyphs: Vec<GlyphSdf>,
    missing: Vec<String>,
    perf: GlyphBatchPerf,
}

#[derive(Serialize)]
struct GlyphBatchPerf {
    total_ms: f64,
    face_load_ms: f64,
    glyph_total_ms: f64,
    glyph_count: usize,
    per_glyph_avg_ms: f64,
    total_pixel_count: usize,
    avg_pixels_per_glyph: f64,
}

#[derive(Serialize)]
struct GlyphBatchError {
    error: String,
}

#[derive(Serialize)]
struct FreeTypeContract<'a> {
    font_engine_fingerprint: &'a str,
    freetype_version: &'a str,
    modules: &'a [&'a str],
    load_contract: &'a str,
}

#[derive(Serialize)]
struct GlyphMapBatch {
    region: String,
    family: String,
    font_source_hash: String,
    glyphs: Vec<GlyphMapEntry>,
    missing: Vec<String>,
}

#[derive(Serialize)]
struct GlyphMapEntry {
    ch: String,
    glyph_index: u32,
}

#[derive(Serialize)]
struct GlyphSdf {
    key: String,
    region: String,
    family: String,
    font_source_hash: String,
    ch: String,
    glyph_index: u32,
    width: usize,
    height: usize,
    bearing_x: f32,
    bearing_y: f32,
    x_offset: f32,
    y_offset: f32,
    advance: f32,
    plane_bearing_x: f32,
    plane_bearing_y: f32,
    plane_width: f32,
    plane_height: f32,
    drawable: bool,
    pixels_base64: String,
}

// ===== Mask batch (for WebGPU SDF path) =====

#[derive(Serialize)]
struct GlyphMaskBatch {
    region: String,
    family: String,
    font_source_hash: String,
    base_size: f32,
    spread: f32,
    glyphs: Vec<GlyphMask>,
    missing: Vec<String>,
    perf: GlyphBatchPerf,
}

#[derive(Serialize)]
struct GlyphMask {
    key: String,
    region: String,
    family: String,
    font_source_hash: String,
    ch: String,
    width: usize,
    height: usize,
    raster_width: usize,
    raster_height: usize,
    supersample: usize,
    bearing_x: f32,
    bearing_y: f32,
    x_offset: f32,
    y_offset: f32,
    advance: f32,
    plane_bearing_x: f32,
    plane_bearing_y: f32,
    plane_width: f32,
    plane_height: f32,
    drawable: bool,
    mask_base64: String,
}

#[no_mangle]
pub unsafe extern "C" fn sdf_layout_freetype_build_mask_json(
    font_ptr: *const u8,
    font_len: usize,
    codepoints_ptr: *const u32,
    codepoints_len: usize,
    region_ptr: *const c_char,
    family_ptr: *const c_char,
    font_source_hash_ptr: *const c_char,
    supersample: usize,
) -> *mut c_char {
    install_panic_hook();
    let result = build_glyph_batch_mask_json(
        font_ptr,
        font_len,
        codepoints_ptr,
        codepoints_len,
        region_ptr,
        family_ptr,
        font_source_hash_ptr,
        supersample.clamp(1, 4),
    );
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

unsafe fn build_glyph_batch_mask_json(
    font_ptr: *const u8,
    font_len: usize,
    codepoints_ptr: *const u32,
    codepoints_len: usize,
    region_ptr: *const c_char,
    family_ptr: *const c_char,
    font_source_hash_ptr: *const c_char,
    supersample: usize,
) -> Result<String, String> {
    if font_ptr.is_null() || codepoints_ptr.is_null() {
        return Err("null font or codepoint pointer".to_string());
    }
    let t0 = Instant::now();
    let font_bytes = slice::from_raw_parts(font_ptr, font_len).to_vec();
    let codepoints = slice::from_raw_parts(codepoints_ptr, codepoints_len);
    let region = read_c_string(region_ptr)?;
    let family = read_c_string(family_ptr)?;
    let font_source_hash = read_c_string(font_source_hash_ptr)?;

    let library = Library::init().map_err(|err| format!("FreeType init failed: {err:?}"))?;
    let t1 = Instant::now();
    let face = library
        .new_memory_face(Rc::new(font_bytes), 0)
        .map_err(|err| format!("load memory face failed: {err:?}"))?;
    face.set_char_size((TMP_POINT_SIZE as isize) * 64, 0, 72, 72)
        .map_err(|err| format!("set char size failed: {err:?}"))?;
    let t2 = Instant::now();

    let mut glyphs = Vec::new();
    let mut missing = Vec::new();
    let mut glyph_total_ms = 0.0f64;
    let mut total_pixel_count: usize = 0;
    for codepoint in codepoints {
        let Some(ch) = char::from_u32(*codepoint) else {
            missing.push(format!("U+{codepoint:04X}"));
            continue;
        };
        if ch == '\n' || ch == '\r' {
            continue;
        }
        let g0 = Instant::now();
        match build_glyph_mask(&face, &region, &family, &font_source_hash, ch, supersample) {
            Ok(glyph) => {
                total_pixel_count += glyph.raster_width * glyph.raster_height;
                glyphs.push(glyph);
            }
            Err(message) => missing.push(format!("{family}:{ch}:{message}")),
        }
        glyph_total_ms += g0.elapsed().as_secs_f64() * 1000.0;
    }
    let t3 = Instant::now();

    let glyph_count = glyphs.len();
    serde_json::to_string(&GlyphMaskBatch {
        region,
        family,
        font_source_hash,
        base_size: TMP_POINT_SIZE,
        spread: TMP_SPREAD,
        glyphs,
        missing,
        perf: GlyphBatchPerf {
            total_ms: duration_ms(t0, t3),
            face_load_ms: duration_ms(t1, t2),
            glyph_total_ms,
            glyph_count,
            per_glyph_avg_ms: if glyph_count > 0 {
                glyph_total_ms / glyph_count as f64
            } else {
                0.0
            },
            total_pixel_count,
            avg_pixels_per_glyph: if glyph_count > 0 {
                total_pixel_count as f64 / glyph_count as f64
            } else {
                0.0
            },
        },
    })
    .map_err(|err| format!("serialize mask batch failed: {err}"))
}

fn build_glyph_mask(
    face: &freetype::Face,
    region: &str,
    family: &str,
    font_source_hash: &str,
    ch: char,
    supersample: usize,
) -> Result<GlyphMask, String> {
    let glyph_id = face
        .get_char_index(ch as usize)
        .ok_or_else(|| "missing cmap entry".to_string())?;
    face.load_glyph(glyph_id, LoadFlag::NO_HINTING)
        .map_err(|err| format!("load glyph failed: {err:?}"))?;

    let glyph = face.glyph();
    let metrics = glyph.metrics();
    let bear_x = metrics.horiBearingX as f32 / 64.0;
    let bear_y = metrics.horiBearingY as f32 / 64.0;
    let met_w = metrics.width as f32 / 64.0;
    let met_h = metrics.height as f32 / 64.0;
    let advance = metrics.horiAdvance as f32 / 64.0;

    let outline = &glyph.raw().outline;
    if outline.n_contours <= 0 || outline.n_points <= 0 {
        return Ok(GlyphMask {
            key: glyph_key(region, family, font_source_hash, ch),
            region: region.to_string(),
            family: family.to_string(),
            font_source_hash: font_source_hash.to_string(),
            ch: ch.to_string(),
            width: 1,
            height: 1,
            raster_width: 1,
            raster_height: 1,
            supersample,
            bearing_x: 0.0,
            bearing_y: 0.0,
            x_offset: 0.0,
            y_offset: 0.0,
            advance,
            plane_bearing_x: 0.0,
            plane_bearing_y: 0.0,
            plane_width: advance.max(1.0 / 64.0),
            plane_height: 0.0,
            drawable: false,
            mask_base64: encode_pixels(&[0]),
        });
    }

    let rect_left_px = bear_x.floor();
    let rect_top_px = bear_y.ceil();
    let rect_right_px = (bear_x + met_w).ceil();
    let rect_bottom_px = (bear_y - met_h).floor();
    let spread_px = TMP_SPREAD.ceil();
    let sample_left_px = rect_left_px - spread_px;
    let sample_top_px = rect_top_px + spread_px;
    let sample_right_px = rect_right_px + spread_px;
    let sample_bottom_px = rect_bottom_px - spread_px;

    let width = (sample_right_px - sample_left_px).max(1.0) as usize;
    let height = (sample_top_px - sample_bottom_px).max(1.0) as usize;

    let ss = supersample.max(1);
    let raster_w = width * ss;
    let raster_h = height * ss;
    glyph
        .render_glyph(RenderMode::Normal)
        .map_err(|err| format!("render glyph failed: {err:?}"))?;
    let bitmap = glyph.bitmap();
    let bm_w = bitmap.width() as usize;
    let bm_h = bitmap.rows() as usize;
    let bm_left = glyph.bitmap_left();
    let bm_top = glyph.bitmap_top();

    let mut mask = vec![0u8; raster_w * raster_h];
    if bm_w > 0 && bm_h > 0 {
        let buffer = bitmap.buffer();
        let pitch = bitmap.pitch().unsigned_abs() as usize;
        for ry in 0..raster_h {
            for rx in 0..raster_w {
                let px_26_6 = (sample_left_px + (rx as f32 + 0.5) / ss as f32) * 64.0;
                let py_26_6 = (sample_top_px - (ry as f32 + 0.5) / ss as f32) * 64.0;
                let bx = ((px_26_6 / 64.0) - bm_left as f32).floor() as isize;
                let by = (bm_top as f32 - (py_26_6 / 64.0)).floor() as isize;
                if bx >= 0 && by >= 0 && (bx as usize) < bm_w && (by as usize) < bm_h {
                    let coverage = buffer[by as usize * pitch + bx as usize];
                    mask[ry * raster_w + rx] = if coverage >= 128 { 1 } else { 0 };
                }
            }
        }
    }

    Ok(GlyphMask {
        key: glyph_key(region, family, font_source_hash, ch),
        region: region.to_string(),
        family: family.to_string(),
        font_source_hash: font_source_hash.to_string(),
        ch: ch.to_string(),
        width,
        height,
        raster_width: raster_w,
        raster_height: raster_h,
        supersample: ss,
        bearing_x: sample_left_px,
        bearing_y: sample_top_px,
        x_offset: sample_left_px,
        y_offset: -sample_top_px,
        advance,
        plane_bearing_x: bear_x,
        plane_bearing_y: bear_y,
        plane_width: met_w.max(1.0 / 64.0),
        plane_height: met_h.max(1.0 / 64.0),
        drawable: true,
        mask_base64: encode_pixels(&mask),
    })
}
