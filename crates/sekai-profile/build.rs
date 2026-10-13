fn main() {
    println!("cargo:rerun-if-changed=native/jpeg_turbo_encoder.c");
    println!("cargo:rerun-if-changed=native/x264_encoder.c");

    if std::env::var_os("CARGO_FEATURE_JPEG_TURBO").is_some() {
        let include = std::env::var_os("DEP_TURBOJPEG_INCLUDE")
            .expect("turbojpeg-sys did not expose its vendored include directory");
        cc::Build::new()
            .file("native/jpeg_turbo_encoder.c")
            .include(include)
            .warnings(true)
            .extra_warnings(true)
            .flag_if_supported("-Werror")
            .compile("allium_jpeg_turbo_encoder");
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-lib=static=jpeg-static");
        } else {
            println!("cargo:rustc-link-lib=static=jpeg");
        }
    }

    if std::env::var_os("CARGO_FEATURE_ANIMATION_EXPORT").is_some() {
        cc::Build::new()
            .file("native/x264_encoder.c")
            .warnings(true)
            .extra_warnings(true)
            .flag_if_supported("-Werror")
            .compile("allium_x264_encoder");
        println!("cargo:rustc-link-lib=dylib=x264");
    }
}
