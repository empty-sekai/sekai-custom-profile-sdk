mod atlas;
mod authoring_runtime;
mod geometry;
mod glyph_plan;
mod layout;
mod masterdata_runtime;
mod scene;
#[cfg(test)]
mod shader_contract;
mod ugui_text;

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::rc::Rc;
use std::slice;
use std::sync::Once;

use base64::Engine;
use freetype::{face::LoadFlag, Library, RenderMode};
use sekai_profile_renderer_core::sdf_glyph::{self, CoverageBitmap, GlyphSdfGrid};
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

/// Lays out uGUI text commands; see [`ugui_text::layout_json`]. `fonts` holds
/// the font files the JSON input names by byte range.
///
/// # Safety
///
/// `fonts_ptr` must point to `fonts_len` readable bytes (it may be null when
/// `fonts_len` is 0), and `input_json_ptr` to `input_json_len` readable bytes.
/// The returned string must be released with `sdf_layout_freetype_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sdf_layout_ugui_text_json(
    fonts_ptr: *const u8,
    fonts_len: usize,
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        let fonts = if fonts_len == 0 {
            &[][..]
        } else if fonts_ptr.is_null() {
            return Err("null font buffer pointer".to_string());
        } else {
            slice::from_raw_parts(fonts_ptr, fonts_len)
        };
        read_utf8_slice(input_json_ptr, input_json_len)
            .and_then(|input| ugui_text::layout_json(fonts, input))
    })
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
    card: sekai_profile_renderer_core::profile_source::CustomProfileCard,
    snapshot: sekai_profile_renderer_core::profile_scene::ProfileResolveSnapshot,
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
            value: sekai_profile_renderer_core::locale::resolve(&request.region, &request.key),
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
        let resolved = sekai_profile_renderer_core::profile_scene::resolve_profile_scene(
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
    let font_bytes = Rc::new(slice::from_raw_parts(font_ptr, font_len).to_vec());
    let codepoints = slice::from_raw_parts(codepoints_ptr, codepoints_len);
    let region = read_c_string(region_ptr)?;
    let family = read_c_string(family_ptr)?;
    let font_source_hash = read_c_string(font_source_hash_ptr)?;

    let library = Library::init().map_err(|err| format!("FreeType init failed: {err:?}"))?;
    let t1 = Instant::now();
    let face = open_memory_face(&library, &font_bytes, TMP_POINT_SIZE)?;
    let raster_face = if supersample > 1 {
        Some(open_memory_face(
            &library,
            &font_bytes,
            TMP_POINT_SIZE * supersample as f32,
        )?)
    } else {
        None
    };
    let raster_face = raster_face.as_ref().unwrap_or(&face);
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
            build_glyph_edt(
                &face,
                raster_face,
                &region,
                &family,
                &font_source_hash,
                ch,
                supersample,
            )
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

/// Glyph metrics in pixels at the sampling point size.
#[derive(Clone, Copy)]
struct SamplingMetrics {
    bearing_x: f32,
    bearing_y: f32,
    width: f32,
    height: f32,
    advance: f32,
}

impl SamplingMetrics {
    fn from_slot(slot: &freetype::GlyphSlot) -> Self {
        let metrics = slot.metrics();
        Self {
            bearing_x: metrics.horiBearingX as f32 / 64.0,
            bearing_y: metrics.horiBearingY as f32 / 64.0,
            width: metrics.width as f32 / 64.0,
            height: metrics.height as f32 / 64.0,
            advance: metrics.horiAdvance as f32 / 64.0,
        }
    }

    fn grid(self) -> GlyphSdfGrid {
        GlyphSdfGrid::from_metrics(
            self.bearing_x,
            self.bearing_y,
            self.width,
            self.height,
            TMP_SPREAD,
        )
    }
}

fn open_memory_face(
    library: &Library,
    font_bytes: &Rc<Vec<u8>>,
    point_size: f32,
) -> Result<freetype::Face, String> {
    let face = library
        .new_memory_face(Rc::clone(font_bytes), 0)
        .map_err(|err| format!("load memory face failed: {err:?}"))?;
    face.set_char_size((point_size * 64.0).round() as isize, 0, 72, 72)
        .map_err(|err| format!("set char size failed: {err:?}"))?;
    Ok(face)
}

fn has_outline(slot: &freetype::GlyphSlot) -> bool {
    let outline = &slot.raw().outline;
    outline.n_contours > 0 && outline.n_points > 0
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

    let slot = face.glyph();
    let metrics = SamplingMetrics::from_slot(slot);
    if !has_outline(slot) {
        return Ok(empty_metric_glyph(
            region,
            family,
            font_source_hash,
            ch,
            glyph_id,
            metrics.advance,
        ));
    }
    let contours = unsafe { extract_segments(&slot.raw().outline) };
    if contours.is_empty() {
        return Ok(empty_metric_glyph(
            region,
            family,
            font_source_hash,
            ch,
            glyph_id,
            metrics.advance,
        ));
    }
    let grid = metrics.grid();
    let pixels = sdf_glyph::analytic_sdf(&contours, grid, TMP_SPREAD);
    Ok(glyph_sdf(
        region,
        family,
        font_source_hash,
        ch,
        glyph_id,
        metrics,
        grid,
        &pixels,
    ))
}

/// `face` is at the sampling point size and supplies the metrics and grid;
/// `raster_face` is at `supersample` times that size, so its coverage bitmap
/// lines up with the grid's supersampled cells.
fn build_glyph_edt(
    face: &freetype::Face,
    raster_face: &freetype::Face,
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

    let slot = face.glyph();
    let metrics = SamplingMetrics::from_slot(slot);
    if !has_outline(slot) {
        return Ok(empty_metric_glyph(
            region,
            family,
            font_source_hash,
            ch,
            glyph_id,
            metrics.advance,
        ));
    }
    let grid = metrics.grid();

    raster_face
        .load_glyph(glyph_id, LoadFlag::NO_HINTING)
        .map_err(|err| format!("load glyph failed: {err:?}"))?;
    let raster = raster_face.glyph();
    raster
        .render_glyph(RenderMode::Normal)
        .map_err(|err| format!("render glyph failed: {err:?}"))?;
    let bitmap = raster.bitmap();
    let width = bitmap.width().max(0) as usize;
    let rows = bitmap.rows().max(0) as usize;
    let coverage = CoverageBitmap {
        buffer: if width > 0 && rows > 0 {
            bitmap.buffer()
        } else {
            &[]
        },
        width,
        rows,
        pitch: bitmap.pitch().unsigned_abs() as usize,
        left: raster.bitmap_left(),
        top: raster.bitmap_top(),
    };
    let pixels = sdf_glyph::edt_sdf(&coverage, grid, supersample, TMP_SPREAD);
    Ok(glyph_sdf(
        region,
        family,
        font_source_hash,
        ch,
        glyph_id,
        metrics,
        grid,
        &pixels,
    ))
}

#[allow(clippy::too_many_arguments)]
fn glyph_sdf(
    region: &str,
    family: &str,
    font_source_hash: &str,
    ch: char,
    glyph_index: u32,
    metrics: SamplingMetrics,
    grid: GlyphSdfGrid,
    pixels: &[u8],
) -> GlyphSdf {
    GlyphSdf {
        key: glyph_key(region, family, font_source_hash, ch),
        region: region.to_string(),
        family: family.to_string(),
        font_source_hash: font_source_hash.to_string(),
        ch: ch.to_string(),
        glyph_index,
        width: grid.width,
        height: grid.height,
        bearing_x: grid.left,
        bearing_y: grid.top,
        x_offset: grid.left,
        y_offset: -grid.top,
        advance: metrics.advance,
        plane_bearing_x: metrics.bearing_x,
        plane_bearing_y: metrics.bearing_y,
        plane_width: metrics.width.max(1.0 / 64.0),
        plane_height: metrics.height.max(1.0 / 64.0),
        drawable: true,
        pixels_base64: encode_pixels(pixels),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FONT: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

    fn pixels(glyph: &GlyphSdf) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(&glyph.pixels_base64)
            .expect("glyph pixels are base64")
    }

    #[test]
    fn supersampled_edt_tracks_the_analytic_field_on_the_same_grid() {
        let Ok(bytes) = std::fs::read(TEST_FONT) else {
            eprintln!("skipping: {TEST_FONT} is not installed");
            return;
        };
        let bytes = Rc::new(bytes);
        let library = Library::init().expect("FreeType");
        let face = open_memory_face(&library, &bytes, TMP_POINT_SIZE).expect("face");
        let raster_face =
            open_memory_face(&library, &bytes, TMP_POINT_SIZE * 4.0).expect("raster face");
        for ch in ['O', 'g'] {
            let exact = build_glyph(&face, "cn", "test", "hash", ch).expect("analytic glyph");
            let edt = build_glyph_edt(&face, &raster_face, "cn", "test", "hash", ch, 4)
                .expect("EDT glyph");
            assert_eq!((edt.width, edt.height), (exact.width, exact.height));
            assert_eq!(
                (edt.bearing_x, edt.bearing_y, edt.advance),
                (exact.bearing_x, exact.bearing_y, exact.advance)
            );
            let (mut sum, mut count) = (0.0f32, 0usize);
            for (&value, &reference) in pixels(&edt).iter().zip(&pixels(&exact)) {
                if reference == 0 || reference == 255 {
                    continue;
                }
                sum += (f32::from(value) - f32::from(reference)).abs();
                count += 1;
            }
            let mean = sum / count as f32;
            assert!(mean < 2.5, "{ch}: ss4 mean gray error {mean}");
        }
    }
}
