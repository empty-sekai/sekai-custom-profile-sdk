//! emscripten 链接驱动壳。
//!
//! C ABI 导出定义在 `lib.rs`。此处用 `#[path]` 把同一份源**直接编进 bin**
//! （而非作为 rlib 依赖链接）——否则 thin-LTO 跨 rlib→bin 边界会把
//! `#[no_mangle]` 符号内部化，导致 wasm-ld 的 `--export=alr_*` 找不到符号。
//! lib 目标仍保留，供 native `cargo check` 复用。
//!
//! emscripten 以 `MODULARIZE` 工厂导出运行时，Module 初始化后不调用 main，
//! 故 main 仅强引用导出符号防止链接前死代码消除。

#[path = "lib.rs"]
mod exports;

/// 强引用导出符号，阻止链接前的死代码消除。
fn keep_exports() {
    let anchors: &[*const ()] = &[
        exports::alr_alloc as *const (),
        exports::alr_free as *const (),
        exports::alr_last_error as *const (),
        exports::alr_load_masterdata as *const (),
        exports::alr_register_font as *const (),
        exports::alr_init as *const (),
        exports::alr_collect_asset_keys as *const (),
        exports::alr_put_asset as *const (),
        exports::alr_render as *const (),
        exports::alr_render_layer_cropped as *const (),
        exports::alr_render_all_layers as *const (),
    ];
    for &f in anchors {
        unsafe {
            core::ptr::read_volatile(&f);
        }
    }
}

fn main() {
    keep_exports();
}
