//! Builds the icon into the Windows exe, so Explorer, a pinned taskbar
//! button and shortcuts show it. The resource file is made from
//! `assets/icon.png` by `tools/make_icon.py`.

#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.res");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if windows && msvc {
        let folder = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
        let res = std::path::Path::new(&folder)
            .join("assets")
            .join("icon.res");
        // The Microsoft linker takes a compiled resource file as an input.
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
