#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
dist_dir="$script_dir/dist"
image="allium-renderer-wasm:0.2.1"
cargo_registry_root="${CARGO_HOME:-$HOME/.cargo}/registry/src"

fetch_freetype_source() {
  if find "$cargo_registry_root" -path '*/freetype-sys-0.23.0/freetype2' -type d -print -quit | grep -q .; then
    return
  fi
  local fetch_root="$script_dir/target/freetype-fetch"
  mkdir -p "$fetch_root/src"
  cat > "$fetch_root/Cargo.toml" <<'EOF'
[package]
name = "allium-freetype-source-fetch"
version = "0.0.0"
edition = "2021"

[dependencies]
freetype-sys = "=0.23.0"

[workspace]
EOF
  printf 'fn main() {}\n' > "$fetch_root/src/main.rs"
  cargo fetch --manifest-path "$fetch_root/Cargo.toml" >/dev/null
}

build_minimal_freetype() {
  local source_root out overlay objects relative object
  source_root="$(find "$cargo_registry_root" -path '*/freetype-sys-0.23.0/freetype2' -type d -print -quit)"
  [[ -n "$source_root" ]] || {
    echo "freetype-sys 0.23.0 source was not fetched" >&2
    exit 1
  }

  out="$script_dir/target/freetype-min"
  overlay="$out/include/freetype/config"
  objects="$out/objects"
  rm -rf "$out"
  mkdir -p "$overlay" "$objects" "$out/lib/pkgconfig"
  cp "$script_dir/freetype-min/ftmodule.h" "$overlay/ftmodule.h"

  sed \
    -e 's/^#define FT_CONFIG_OPTION_USE_LZW/\/\* allium-min: FT_CONFIG_OPTION_USE_LZW \*\//' \
    -e 's/^#define FT_CONFIG_OPTION_USE_ZLIB/\/\* allium-min: FT_CONFIG_OPTION_USE_ZLIB \*\//' \
    -e 's/^#define FT_CONFIG_OPTION_SVG/\/\* allium-min: FT_CONFIG_OPTION_SVG \*\//' \
    -e 's/^#define FT_CONFIG_OPTION_MAC_FONTS/\/\* allium-min: FT_CONFIG_OPTION_MAC_FONTS \*\//' \
    -e 's/^#define FT_CONFIG_OPTION_GUESSING_EMBEDDED_RFORK/\/\* allium-min: FT_CONFIG_OPTION_GUESSING_EMBEDDED_RFORK \*\//' \
    -e 's/^#define FT_CONFIG_OPTION_INCREMENTAL/\/\* allium-min: FT_CONFIG_OPTION_INCREMENTAL \*\//' \
    -e 's/^#define TT_CONFIG_OPTION_EMBEDDED_BITMAPS/\/\* allium-min: TT_CONFIG_OPTION_EMBEDDED_BITMAPS \*\//' \
    -e 's/^#define TT_CONFIG_OPTION_COLOR_LAYERS/\/\* allium-min: TT_CONFIG_OPTION_COLOR_LAYERS \*\//' \
    -e 's/^#define TT_CONFIG_OPTION_BYTECODE_INTERPRETER/\/\* allium-min: TT_CONFIG_OPTION_BYTECODE_INTERPRETER \*\//' \
    -e 's/^#define TT_CONFIG_OPTION_SUBPIXEL_HINTING/\/\* allium-min: TT_CONFIG_OPTION_SUBPIXEL_HINTING \*\//' \
    -e 's/^#define TT_CONFIG_OPTION_BDF/\/\* allium-min: TT_CONFIG_OPTION_BDF \*\//' \
    "$source_root/include/freetype/config/ftoption.h" > "$overlay/ftoption.h"

  local sources=(
    base/ftbase.c
    base/ftdebug.c
    base/ftinit.c
    base/ftmm.c
    base/ftsystem.c
    truetype/truetype.c
    sfnt/sfnt.c
    cff/cff.c
    psaux/psaux.c
    psnames/psnames.c
    smooth/smooth.c
  )
  for relative in "${sources[@]}"; do
    object="$objects/${relative//\//_}.o"
    emcc -O3 -fwasm-exceptions -fvisibility=hidden -ffunction-sections -fdata-sections \
      -D__WASM_SJLJ__ -DFT2_BUILD_LIBRARY \
      -I"$out/include" -I"$source_root/include" -I"$source_root/src" \
      -c "$source_root/src/$relative" -o "$object"
  done
  emar crs "$out/lib/libfreetype2.a" "$objects"/*.o

  cat > "$out/lib/pkgconfig/freetype2.pc" <<EOF
prefix=$out
libdir=\${prefix}/lib
includedir=$source_root/include

Name: FreeType 2 (Allium minimal)
Description: Minimal TrueType/CFF/smooth FreeType build for Allium renderer WASM
Version: 26.1.20
Libs: -L\${libdir} -lfreetype2
Cflags: -I$out/include -I\${includedir}
EOF
  printf '%s\n' "$out"
}

build_inside_container() {
  local output_dir="${1:-/artifacts}"
  command -v cargo >/dev/null || { echo "cargo was not found" >&2; exit 1; }
  if ! command -v emcc >/dev/null 2>&1; then
    local emsdk_env
    for emsdk_env in /emsdk/emsdk_env.sh /opt/emsdk/emsdk_env.sh; do
      if [[ -f "$emsdk_env" ]]; then
        # shellcheck disable=SC1090
        source "$emsdk_env" >/dev/null
        break
      fi
    done
  fi
  command -v emcc >/dev/null || { echo "emcc was not found" >&2; exit 1; }

  cd "$script_dir"
  fetch_freetype_source
  local freetype_min
  freetype_min="$(build_minimal_freetype)"
  local freetype_source
  freetype_source="$(find "$cargo_registry_root" -path '*/freetype-sys-0.23.0/freetype2' -type d -print -quit)"
  export PKG_CONFIG_ALLOW_CROSS=1
  export PKG_CONFIG_PATH="$freetype_min/lib/pkgconfig"
  export FREETYPE2_STATIC=1
  export CFLAGS_wasm32_unknown_emscripten="-D__WASM_SJLJ__ -fwasm-exceptions"
  export ALLIUM_FONT_ENGINE_FINGERPRINT="freetype-2.13.2:min-v1"
  export CARGO_TARGET_DIR="$script_dir/target/browser-runtime"

  rm -rf "${output_dir:?}"
  mkdir -p "$output_dir"

  local emscripten_root
  emscripten_root="$(dirname "$(command -v emcc)")"
  mkdir -p target/emscripten
  emcc -O3 -fwasm-exceptions -D__WASM_SJLJ__ \
    -I"$emscripten_root/system/lib/libc" \
    -c "$emscripten_root/system/lib/compiler-rt/emscripten_setjmp.c" \
    -o target/emscripten/emscripten_setjmp.o

  export RUSTFLAGS="-C link-arg=-sMODULARIZE=1 -C link-arg=-sEXPORT_ES6=1 -C link-arg=-sEXPORT_NAME=createAlliumRenderer -C link-arg=-sALLOW_MEMORY_GROWTH=1 -C link-arg=-sSUPPORT_LONGJMP=wasm -C link-arg=-sENVIRONMENT=web,worker,node -C link-arg=-sEXPORTED_FUNCTIONS=['_main','_malloc','_free','_sdf_layout_freetype_probe','_sdf_layout_freetype_contract_json','_sdf_layout_freetype_map_glyphs_json','_sdf_layout_freetype_build_glyph_json','_sdf_layout_freetype_build_glyph_json_edt','_sdf_layout_freetype_build_mask_json','_sdf_layout_freetype_build_layout_json','_sdf_layout_freetype_glyph_demand_json','_sdf_atlas_create_json','_sdf_atlas_resolve_json','_sdf_atlas_pages_since_json','_sdf_atlas_page_pixels_ptr','_sdf_atlas_page_pixels_len','_sdf_atlas_release','_sdf_atlas_destroy','_sdf_renderer_authoring_create_blank_json','_sdf_renderer_authoring_import_profile_json','_sdf_renderer_authoring_restore_checkpoint_json','_sdf_renderer_authoring_apply_json','_sdf_renderer_authoring_select_json','_sdf_renderer_authoring_begin_gesture_json','_sdf_renderer_authoring_preview_gesture_json','_sdf_renderer_authoring_commit_gesture_json','_sdf_renderer_authoring_cancel_gesture_json','_sdf_renderer_authoring_append_page_json','_sdf_renderer_authoring_duplicate_page_json','_sdf_renderer_authoring_delete_page_json','_sdf_renderer_authoring_move_page_json','_sdf_renderer_authoring_undo_json','_sdf_renderer_authoring_redo_json','_sdf_renderer_authoring_export_json','_sdf_renderer_authoring_checkpoint_json','_sdf_renderer_authoring_destroy','_sdf_renderer_core_scene_create_json','_sdf_renderer_core_profile_scene_create_json','_sdf_renderer_core_masterdata_create_json','_sdf_renderer_core_masterdata_put_table_json','_sdf_renderer_core_masterdata_seal_json','_sdf_renderer_core_profile_prepare_json','_sdf_renderer_core_profile_create_json','_sdf_renderer_core_masterdata_stats_json','_sdf_renderer_core_masterdata_destroy','_sdf_renderer_core_scene_advance_json','_sdf_renderer_core_scene_advance_binary','_sdf_renderer_core_scene_set_mask_json','_sdf_renderer_core_scene_set_masks_json','_sdf_renderer_core_scene_set_tab_json','_sdf_renderer_core_scene_scroll_json','_sdf_renderer_core_scene_dump_json','_sdf_renderer_core_scene_destroy','_sdf_renderer_core_resolve_locale_json','_sdf_renderer_core_resolve_profile_json','_sdf_layout_freetype_free_string'] -C link-arg=-sEXPORTED_RUNTIME_METHODS=['ccall','HEAPU8','HEAPU32'] -C link-arg=-sEXIT_RUNTIME=0 -C link-arg=-Wl,-Map=$script_dir/target/renderer-wasm.map -C link-arg=$script_dir/target/emscripten/emscripten_setjmp.o"
  RUSTFLAGS="${RUSTFLAGS/]/,'_sdf_layout_freetype_plan_glyphs_json']}"
  RUSTFLAGS="${RUSTFLAGS/]/,'_sdf_renderer_authoring_elements_json']}"
  export RUSTFLAGS
  cargo build --locked --target wasm32-unknown-emscripten --release --bin allium_renderer_wasm

  cp "$CARGO_TARGET_DIR/wasm32-unknown-emscripten/release/allium_renderer_wasm.js" "$output_dir/"
  cp "$CARGO_TARGET_DIR/wasm32-unknown-emscripten/release/allium_renderer_wasm.wasm" "$output_dir/"
  mkdir -p "$output_dir/third-party/freetype"
  cp "$freetype_source/docs/FTL.TXT" "$output_dir/third-party/freetype/FTL.txt"
}

if [[ "${1:-}" == "--inside-container" ]]; then
  build_inside_container "${2:-/artifacts}"
  exit 0
fi

if ! command -v docker >/dev/null 2>&1 && command -v emcc >/dev/null 2>&1; then
  echo "==> Building directly in the configured development container"
  build_inside_container "$dist_dir"
  echo "==> Browser runtime artifacts"
  ls -la "$dist_dir"
  exit 0
fi

proxy_args=(
  --build-arg HTTP_PROXY=
  --build-arg HTTPS_PROXY=
  --build-arg http_proxy=
  --build-arg https_proxy=
  --build-arg NO_PROXY='*'
)

echo "==> Building the semantic FreeType WASM runtime"
docker build "${proxy_args[@]}" -f "$script_dir/Dockerfile" -t "$image" "$repo_root"

echo "==> Extracting artifacts to $dist_dir"
mkdir -p "$dist_dir"
container_id="$(docker create "$image")"
trap 'docker rm -f "$container_id" >/dev/null 2>&1 || true' EXIT
docker cp "$container_id:/artifacts/allium_renderer_wasm.js" "$dist_dir/"
docker cp "$container_id:/artifacts/allium_renderer_wasm.wasm" "$dist_dir/"
mkdir -p "$dist_dir/third-party"
docker cp "$container_id:/artifacts/third-party/freetype" "$dist_dir/third-party/"

echo "==> Browser runtime artifacts"
ls -la "$dist_dir"
