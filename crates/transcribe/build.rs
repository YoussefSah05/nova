//! whisper.cpp's Metal backend guards newer APIs with `@available`. When the
//! deployment target is older than the SDK (release bundles set
//! `MACOSX_DEPLOYMENT_TARGET` from `minimumSystemVersion`), clang lowers those
//! checks to `__isPlatformVersionAtLeast`, a compiler-rt builtin. rustc links
//! with `-nodefaultlibs`, so link clang's builtins archive explicitly.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let Some(dir) = Command::new("clang")
        .arg("--print-resource-dir")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("lib/darwin"))
        .filter(|dir| dir.join("libclang_rt.osx.a").is_file())
    else {
        println!("cargo:warning=clang builtins not found; release links may miss __isPlatformVersionAtLeast");
        return;
    };
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
