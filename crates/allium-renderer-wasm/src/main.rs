//! Emscripten link driver for the browser renderer runtime.
//!
//! The C ABI is compiled directly into this binary so link-time optimization
//! cannot internalize the exported symbols across an rlib boundary.

#[path = "lib.rs"]
mod exports;

fn keep_exports() {
    let anchors: &[*const ()] = &[
        exports::sdf_layout_freetype_probe as *const (),
        exports::sdf_layout_freetype_contract_json as *const (),
        exports::sdf_layout_freetype_map_glyphs_json as *const (),
        exports::sdf_layout_freetype_plan_glyphs_json as *const (),
        exports::sdf_layout_freetype_build_glyph_json as *const (),
        exports::sdf_layout_freetype_build_glyph_json_edt as *const (),
        exports::sdf_layout_freetype_build_mask_json as *const (),
        exports::sdf_layout_freetype_build_layout_json as *const (),
        exports::sdf_layout_freetype_glyph_demand_json as *const (),
        exports::sdf_atlas_create_json as *const (),
        exports::sdf_atlas_resolve_json as *const (),
        exports::sdf_atlas_pages_since_json as *const (),
        exports::sdf_atlas_page_pixels_ptr as *const (),
        exports::sdf_atlas_page_pixels_len as *const (),
        exports::sdf_atlas_release as *const (),
        exports::sdf_atlas_destroy as *const (),
        exports::sdf_renderer_core_scene_create_json as *const (),
        exports::sdf_renderer_core_profile_scene_create_json as *const (),
        exports::sdf_renderer_core_masterdata_create_json as *const (),
        exports::sdf_renderer_core_masterdata_put_table_json as *const (),
        exports::sdf_renderer_core_masterdata_seal_json as *const (),
        exports::sdf_renderer_core_profile_prepare_json as *const (),
        exports::sdf_renderer_core_profile_create_json as *const (),
        exports::sdf_renderer_core_masterdata_stats_json as *const (),
        exports::sdf_renderer_core_masterdata_destroy as *const (),
        exports::sdf_renderer_core_scene_advance_json as *const (),
        exports::sdf_renderer_core_scene_advance_binary as *const (),
        exports::sdf_renderer_core_scene_set_mask_json as *const (),
        exports::sdf_renderer_core_scene_set_masks_json as *const (),
        exports::sdf_renderer_core_scene_set_tab_json as *const (),
        exports::sdf_renderer_core_scene_scroll_json as *const (),
        exports::sdf_renderer_core_scene_dump_json as *const (),
        exports::sdf_renderer_core_scene_destroy as *const (),
        exports::sdf_renderer_core_resolve_locale_json as *const (),
        exports::sdf_renderer_core_resolve_profile_json as *const (),
        exports::sdf_layout_freetype_free_string as *const (),
    ];
    for &function in anchors {
        unsafe { core::ptr::read_volatile(&function) };
    }
}

fn main() {
    keep_exports();
}
