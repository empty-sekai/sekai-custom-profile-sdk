//! FreeType, SDF and scene runtime behind the browser package.
//!
//! This crate is compiled to WebAssembly and shipped inside
//! `@empty-sekai/sekai-custom-profile-sdk`; it is not published as a registry
//! crate. The TypeScript wrapper drives it across a C ABI rather than through
//! Rust types, so every export is `extern "C"` and every value crosses as a
//! JSON string or a raw pointer with a paired length.
//!
//! # Calling convention
//!
//! Each group of exports follows the same shape:
//!
//! - A `*_create_json` or `*_import_*_json` call allocates a handle; the
//!   matching `*_destroy` releases it. Handles are `u32` and an unknown handle
//!   returns a negative status rather than trapping.
//! - Calls that return data return a `*mut c_char` holding a
//!   NUL-terminated JSON string owned by the caller's side of the boundary.
//!   The string is released with [`sdf_layout_freetype_free_string`] and never
//!   with the module's own allocator.
//! - Calls that return bytes (`*_binary`, atlas pages) write into a buffer the
//!   caller sized from a preceding length query, or expose a pointer and length
//!   pair that stays valid until the next call on the same handle.
//!
//! The entry points are `unsafe` because they dereference the caller's
//! pointers; the TypeScript wrapper is what guarantees those pointers and
//! lengths came from the length query for the same handle.
//!
//! # Modules
//!
//! The contents are private: `scene` holds the scene and masterdata runtimes,
//! `authoring_runtime` the authoring session state, `atlas` the SDF atlas
//! handles, `layout` and `ugui_text` the text layout entry points, `glyph_plan`
//! the glyph demand planner, `geometry` the outline conversion, and
//! `shader_contract` the tests that keep the GLSL shaders and the Rust-side
//! material parameters in agreement.

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
use sekai_profile_core::sdf_glyph::{self, CoverageBitmap, GlyphSdfGrid};
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

/// Checks that FreeType initializes in this module: `1` on success, `-1`
/// when FreeType cannot be initialized.
#[no_mangle]
pub extern "C" fn sdf_layout_freetype_probe() -> i32 {
    install_panic_hook();
    match Library::init() {
        Ok(_) => 1,
        Err(_) => -1,
    }
}
/// Returns the FreeType build contract as JSON: `font_engine_fingerprint`,
/// `freetype_version`, the compiled `modules` and the glyph `load_contract`.
/// The returned string must be released with
/// [`sdf_layout_freetype_free_string`].
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

/// # Safety
///
/// `font_ptr` must point to `font_len` readable bytes and `codepoints_ptr` to
/// `codepoints_len` readable, 4-byte aligned `u32` values, both valid for the
/// call; null for either is rejected with an error result. `region_ptr`,
/// `family_ptr` and `font_source_hash_ptr` must each be null or point to a
/// NUL-terminated string.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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
        RawSlice::new(font_ptr, font_len),
        RawSlice::new(codepoints_ptr, codepoints_len),
        RawFontIdentity {
            region: region_ptr,
            family: family_ptr,
            font_source_hash: font_source_hash_ptr,
        },
        0,
    );
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

/// # Safety
///
/// `font_ptr` must point to `font_len` readable bytes and `codepoints_ptr` to
/// `codepoints_len` readable, 4-byte aligned `u32` values, both valid for the
/// call; null for either is rejected with an error result. `region_ptr`,
/// `family_ptr` and `font_source_hash_ptr` must each be null or point to a
/// NUL-terminated string.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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
        RawSlice::new(font_ptr, font_len),
        RawSlice::new(codepoints_ptr, codepoints_len),
        RawFontIdentity {
            region: region_ptr,
            family: family_ptr,
            font_source_hash: font_source_hash_ptr,
        },
        supersample.clamp(1, 4),
    );
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

/// # Safety
///
/// `font_ptr` must point to `font_len` readable bytes and `codepoints_ptr` to
/// `codepoints_len` readable, 4-byte aligned `u32` values, both valid for the
/// call; null for either is rejected with an error result. `region_ptr`,
/// `family_ptr` and `font_source_hash_ptr` must each be null or point to a
/// NUL-terminated string.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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
        RawSlice::new(font_ptr, font_len),
        RawSlice::new(codepoints_ptr, codepoints_len),
        RawFontIdentity {
            region: region_ptr,
            family: family_ptr,
            font_source_hash: font_source_hash_ptr,
        },
    );
    into_c_string(result.unwrap_or_else(|message| {
        serde_json::to_string(&GlyphBatchError { error: message })
            .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
    }))
}

/// # Safety
///
/// `font_ptr` must point to `font_len` readable bytes and `codepoints_ptr` to
/// `codepoints_len` readable, 4-byte aligned `u32` values, both valid for the
/// call; null for either is rejected with an error result. `region_ptr`,
/// `family_ptr` and `font_source_hash_ptr` must each be null or point to a
/// NUL-terminated string.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
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

/// Lays out uGUI text commands; see `ugui_text::layout_json`. `fonts` holds
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// Pointer to the one-byte-per-texel pixels of atlas page `page`, or null
/// for an unknown handle or page. Read [`sdf_atlas_page_pixels_len`] bytes
/// from it; the pointer stays valid until the next call on the same handle.
#[no_mangle]
pub extern "C" fn sdf_atlas_page_pixels_ptr(handle: u32, page: usize) -> *const u8 {
    atlas::page_pixels(handle, page)
        .map(|(pointer, _)| pointer)
        .unwrap_or(std::ptr::null())
}

/// Length in bytes of the pixels [`sdf_atlas_page_pixels_ptr`] points to, or
/// `0` for an unknown handle or page.
#[no_mangle]
pub extern "C" fn sdf_atlas_page_pixels_len(handle: u32, page: usize) -> usize {
    atlas::page_pixels(handle, page)
        .map(|(_, length)| length)
        .unwrap_or(0)
}

/// Releases a page lease a resolve call returned, unpinning its pages so they
/// can be evicted again. Returns `1` when the lease was held, `0` when it was
/// unknown, and `-1` for an unknown atlas handle.
#[no_mangle]
pub extern "C" fn sdf_atlas_release(handle: u32, lease: u32) -> i32 {
    match atlas::release(handle, lease) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

/// Destroys an atlas handle. Returns `1` when it existed and `0` otherwise.
#[no_mangle]
pub extern "C" fn sdf_atlas_destroy(handle: u32) -> i32 {
    if atlas::destroy(handle) {
        1
    } else {
        0
    }
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_scene_create_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| read_utf8_slice(input_json_ptr, input_json_len).and_then(scene::create))
}

/// Opens an authoring session on a blank profile document and returns
/// `{ handle, revision, document }` as JSON, or `{ "error": ... }`. The
/// returned string must be released with
/// [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_create_blank_json() -> *mut c_char {
    core_json_call(authoring_runtime::create_blank)
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_authoring_import_profile_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len).and_then(authoring_runtime::import_profile)
    })
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// Lists the elements of an authoring session with their IDs, pages,
/// categories and indices, as JSON. The returned string must be released
/// with [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_elements_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::elements(handle))
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// Commits the active gesture as one undoable edit and returns the resulting
/// delta as JSON, or an error when no gesture is active. The returned string
/// must be released with [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_commit_gesture_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::commit_gesture(handle))
}

/// Cancels the active gesture, restoring the document to its state before the
/// gesture, and returns the delta as JSON. The returned string must be
/// released with [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_cancel_gesture_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::cancel_gesture(handle))
}

/// Appends a blank page to the document as one undoable edit and returns the
/// delta as JSON. The returned string must be released with
/// [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_append_page_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::append_page(handle))
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// Undoes the latest edit and returns its delta as JSON, or `null` when there
/// is nothing to undo. The returned string must be released with
/// [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_undo_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::undo(handle))
}

/// Redoes the latest undone edit and returns its delta as JSON, or `null`
/// when there is nothing to redo. The returned string must be released with
/// [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_redo_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::redo(handle))
}

/// Exports the session's document as game profile JSON. The returned string
/// must be released with [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_export_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::export(handle))
}

/// Exports a checkpoint of the session (document, revision, undo and redo
/// history, selection) as JSON, which
/// [`sdf_renderer_authoring_restore_checkpoint_json`] restores. The returned
/// string must be released with [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_checkpoint_json(handle: u32) -> *mut c_char {
    core_json_call(|| authoring_runtime::checkpoint(handle))
}

/// Destroys an authoring session. Returns `1` when it existed and `0`
/// otherwise.
#[no_mangle]
pub extern "C" fn sdf_renderer_authoring_destroy(handle: u32) -> i32 {
    if authoring_runtime::destroy(handle) {
        1
    } else {
        0
    }
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_profile_scene_create_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len).and_then(scene::create_resolved_profile)
    })
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_masterdata_create_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        read_utf8_slice(input_json_ptr, input_json_len).and_then(masterdata_runtime::create)
    })
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// Seals a masterdata session: no more tables can be added, and profile
/// preparation and scene creation become available. Returns the session
/// stats as JSON. The returned string must be released with
/// [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_core_masterdata_seal_json(handle: u32) -> *mut c_char {
    core_json_call(|| masterdata_runtime::seal(handle))
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// Returns a masterdata session's `handle`, `region`, `revision`, `sealed`
/// state and loaded `tables` as JSON. The returned string must be released
/// with [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_core_masterdata_stats_json(handle: u32) -> *mut c_char {
    core_json_call(|| masterdata_runtime::stats(handle))
}

/// Destroys a masterdata session. Returns `1` when it existed and `0`
/// otherwise.
#[no_mangle]
pub extern "C" fn sdf_renderer_core_masterdata_destroy(handle: u32) -> i32 {
    if masterdata_runtime::destroy(handle) {
        1
    } else {
        0
    }
}

/// Advances a scene to `tick` and returns the resulting delta as JSON. The
/// returned string must be released with
/// [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_core_scene_advance_json(handle: u32, tick: u32) -> *mut c_char {
    core_json_call(|| scene::advance(handle, tick as u64))
}

/// Writes the binary delta for `handle` at `tick` into `output` and returns
/// the number of bytes written, or `0` on any error.
///
/// # Safety
///
/// `output` must be null or point to `capacity` writable bytes of linear
/// memory that nothing else references during the call. `capacity` must cover
/// the scene's `40 + layer_count * 24`-byte worst case; a smaller buffer is
/// rejected and left untouched.
#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_scene_advance_binary(
    handle: u32,
    tick: u32,
    output: *mut u8,
    capacity: usize,
) -> usize {
    // SAFETY: `output` and `capacity` come unchanged from this export's
    // caller, whose contract above is the one `scene::advance_binary` requires.
    unsafe { scene::advance_binary(handle, tick as u64, output, capacity) }.unwrap_or_default()
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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
/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. An unknown `handle` yields an error result rather than
/// undefined behaviour.
/// The returned string must be released with
/// `sdf_layout_freetype_free_string`.
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

/// Returns a snapshot of a scene as JSON: its layer table and tree,
/// revisions, interaction regions and telemetry. The returned string must be
/// released with [`sdf_layout_freetype_free_string`].
#[no_mangle]
pub extern "C" fn sdf_renderer_core_scene_dump_json(handle: u32) -> *mut c_char {
    core_json_call(|| scene::dump(handle))
}

/// Destroys a scene. Returns `1` when it existed and `0` otherwise.
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
    card: sekai_profile_core::profile_source::CustomProfileCard,
    snapshot: sekai_profile_core::profile_scene::ProfileResolveSnapshot,
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
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
            value: sekai_profile_core::locale::resolve(&request.region, &request.key),
            region: request.region,
            key: request.key,
        })
        .map_err(|error| error.to_string())
    })
}

/// # Safety
///
/// `input_json_ptr` must point to `input_json_len` readable bytes of UTF-8
/// JSON that stay unmodified for the call; a null pointer is rejected with an
/// error result. The returned string must be released with `sdf_layout_freetype_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sdf_renderer_core_resolve_profile_json(
    input_json_ptr: *const u8,
    input_json_len: usize,
) -> *mut c_char {
    core_json_call(|| {
        let input = read_utf8_slice(input_json_ptr, input_json_len)?;
        let request: ProfileResolveRequest =
            serde_json::from_str(input).map_err(|error| error.to_string())?;
        let resolved = sekai_profile_core::profile_scene::resolve_profile_scene(
            &request.card,
            &request.document_key,
            &request.snapshot,
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&resolved).map_err(|error| error.to_string())
    })
}

/// # Safety
///
/// `ptr` must be null or a string returned by one of this module's exports
/// that has not been released yet; it is freed here and must not be used
/// afterwards.
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

/// # Safety
///
/// `font.ptr` must point to `font.len` readable bytes and `codepoints.ptr` to
/// `codepoints.len` readable, 4-byte aligned `u32` values; both are checked
/// for null and copied before FreeType sees them, so they only need to stay
/// valid for the call. The `raw_identity` pointers must each be null or point
/// to a NUL-terminated string, as [`RawFontIdentity::read`] requires.
unsafe fn build_glyph_batch_json(
    font: RawSlice<u8>,
    codepoints: RawSlice<u32>,
    raw_identity: RawFontIdentity,
    supersample: usize,
) -> Result<String, String> {
    if font.ptr.is_null() || codepoints.ptr.is_null() {
        return Err("null font or codepoint pointer".to_string());
    }
    let t0 = Instant::now();
    let font_bytes = Rc::new(font.as_slice().to_vec());
    let codepoints = codepoints.as_slice();
    let identity = raw_identity.read()?;

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
            build_glyph_edt(&face, raster_face, &identity, ch, supersample)
                .or_else(|_| build_glyph(&face, &identity, ch))
        } else {
            build_glyph(&face, &identity, ch)
        };
        match glyph_result {
            Ok(glyph) => {
                total_pixel_count += glyph.width * glyph.height;
                glyphs.push(glyph);
            }
            Err(message) => missing.push(format!("{}:{ch}:{message}", identity.family)),
        }
        glyph_total_ms += g0.elapsed().as_secs_f64() * 1000.0;
    }
    let t3 = Instant::now();

    let glyph_count = glyphs.len();
    let FontIdentity {
        region,
        family,
        font_source_hash,
    } = identity;
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

/// # Safety
///
/// `font.ptr` must point to `font.len` readable bytes and `codepoints.ptr` to
/// `codepoints.len` readable, 4-byte aligned `u32` values, both valid for the
/// call; null for either is rejected and the font bytes are copied into the
/// face. The `raw_identity` pointers must each be null or point to a
/// NUL-terminated string, as [`RawFontIdentity::read`] requires.
unsafe fn map_glyphs_json(
    font: RawSlice<u8>,
    codepoints: RawSlice<u32>,
    raw_identity: RawFontIdentity,
) -> Result<String, String> {
    if font.ptr.is_null() || codepoints.ptr.is_null() {
        return Err("null font or codepoint pointer".to_string());
    }
    let font_bytes = font.as_slice().to_vec();
    let codepoints = codepoints.as_slice();
    let FontIdentity {
        region,
        family,
        font_source_hash,
    } = raw_identity.read()?;
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

/// # Safety
///
/// `ptr` must be null, which yields an empty string, or point to a
/// NUL-terminated byte string that is readable up to and including its NUL and
/// not written to during the call. The bytes are copied into the returned
/// `String`.
unsafe fn read_c_string(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Ok(String::new());
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map(str::to_string)
        .map_err(|err| format!("invalid utf8 string: {err}"))
}

/// A C array passed across the module boundary: `len` elements at `ptr`.
#[derive(Clone, Copy)]
struct RawSlice<T> {
    ptr: *const T,
    len: usize,
}

impl<T> RawSlice<T> {
    fn new(ptr: *const T, len: usize) -> Self {
        Self { ptr, len }
    }

    /// # Safety
    ///
    /// `ptr` must be non-null, aligned for `T` and point to `len` readable
    /// elements, with `len * size_of::<T>() <= isize::MAX`, that remain valid
    /// and unmodified for the caller-chosen lifetime `'a`.
    unsafe fn as_slice<'a>(&self) -> &'a [T] {
        slice::from_raw_parts(self.ptr, self.len)
    }
}

/// The NUL-terminated strings that name a font across the module boundary.
#[derive(Clone, Copy)]
struct RawFontIdentity {
    region: *const c_char,
    family: *const c_char,
    font_source_hash: *const c_char,
}

impl RawFontIdentity {
    /// Copies the three strings in field order; a null pointer reads as an
    /// empty string.
    ///
    /// # Safety
    ///
    /// Each pointer must be null or point to a NUL-terminated string, as
    /// [`read_c_string`] requires.
    unsafe fn read(self) -> Result<FontIdentity, String> {
        Ok(FontIdentity {
            region: read_c_string(self.region)?,
            family: read_c_string(self.family)?,
            font_source_hash: read_c_string(self.font_source_hash)?,
        })
    }
}

/// The font a glyph export reads: its region, family name and source hash.
/// Every glyph key built from the font starts with these three values.
struct FontIdentity {
    region: String,
    family: String,
    font_source_hash: String,
}

/// # Safety
///
/// A null `ptr` is rejected with an error. Otherwise `ptr` must point to `len`
/// readable bytes, with `len <= isize::MAX`, that remain valid and unmodified
/// for the caller-chosen lifetime `'a`, because the returned `&'a str`
/// borrows them in place.
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

fn build_glyph(face: &freetype::Face, font: &FontIdentity, ch: char) -> Result<GlyphSdf, String> {
    let glyph_id = face
        .get_char_index(ch as usize)
        .ok_or_else(|| "missing cmap entry".to_string())?;
    face.load_glyph(glyph_id, LoadFlag::NO_BITMAP | LoadFlag::NO_HINTING)
        .map_err(|err| format!("load glyph failed: {err:?}"))?;

    let slot = face.glyph();
    let metrics = SamplingMetrics::from_slot(slot);
    if !has_outline(slot) {
        return Ok(empty_metric_glyph(font, ch, glyph_id, metrics.advance));
    }
    // SAFETY: `slot` borrows `face`, whose `load_glyph` above filled it, so the
    // outline's point, tag and contour arrays are alive until the next load on
    // this face; `has_outline` has just confirmed both counts are positive.
    let contours = unsafe { extract_segments(&slot.raw().outline) };
    if contours.is_empty() {
        return Ok(empty_metric_glyph(font, ch, glyph_id, metrics.advance));
    }
    let grid = metrics.grid();
    let pixels = sdf_glyph::analytic_sdf(&contours, grid, TMP_SPREAD);
    Ok(glyph_sdf(font, ch, glyph_id, metrics, grid, &pixels))
}

/// `face` is at the sampling point size and supplies the metrics and grid;
/// `raster_face` is at `supersample` times that size, so its coverage bitmap
/// lines up with the grid's supersampled cells.
fn build_glyph_edt(
    face: &freetype::Face,
    raster_face: &freetype::Face,
    font: &FontIdentity,
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
        return Ok(empty_metric_glyph(font, ch, glyph_id, metrics.advance));
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
    Ok(glyph_sdf(font, ch, glyph_id, metrics, grid, &pixels))
}

fn glyph_sdf(
    font: &FontIdentity,
    ch: char,
    glyph_index: u32,
    metrics: SamplingMetrics,
    grid: GlyphSdfGrid,
    pixels: &[u8],
) -> GlyphSdf {
    GlyphSdf {
        key: glyph_key(&font.region, &font.family, &font.font_source_hash, ch),
        region: font.region.clone(),
        family: font.family.clone(),
        font_source_hash: font.font_source_hash.clone(),
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

fn empty_metric_glyph(font: &FontIdentity, ch: char, glyph_index: u32, advance: f32) -> GlyphSdf {
    GlyphSdf {
        key: glyph_key(&font.region, &font.family, &font.font_source_hash, ch),
        region: font.region.clone(),
        family: font.family.clone(),
        font_source_hash: font.font_source_hash.clone(),
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
        let font = FontIdentity {
            region: "cn".into(),
            family: "test".into(),
            font_source_hash: "hash".into(),
        };
        for ch in ['O', 'g'] {
            let exact = build_glyph(&face, &font, ch).expect("analytic glyph");
            let edt = build_glyph_edt(&face, &raster_face, &font, ch, 4).expect("EDT glyph");
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
