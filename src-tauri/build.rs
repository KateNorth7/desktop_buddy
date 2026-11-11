fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/macos.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .flag("-mmacosx-version-min=14.5")
            .compile("buddy_macos");
        for framework in [
            "AppKit",
            "ApplicationServices",
            "Security",
            "Carbon",
            "Speech",
            "CoreAudio",
            "ScreenCaptureKit",
            "Vision",
            "ImageIO",
            "CoreML",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        println!("cargo:rerun-if-changed=native/macos.m");
        println!("cargo:rerun-if-changed=native/visible_text_walk.h");
        println!("cargo:rerun-if-changed=native/capture_window_selection.h");
    }
    tauri_build::build();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Mock-runtime tests also import Common Controls v6. Tauri links its manifest
        // only to bins by default; include those resources in the backend test EXE too.
        let resources =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("resource.lib");
        println!("cargo:rustc-link-arg-tests={}", resources.display());
    }
}
