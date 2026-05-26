use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    if let Ok(output) = Command::new("pkg-config").args(["--libs", "mpv"]).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for part in stdout.split_whitespace() {
                if let Some(path) = part.strip_prefix("-L") {
                    println!("cargo:rustc-link-search=native={path}");
                } else if let Some(lib) = part.strip_prefix("-l") {
                    println!("cargo:rustc-link-lib={lib}");
                }
            }
            return;
        }
    }

    for path in ["/opt/homebrew/lib", "/usr/local/lib"] {
        if Path::new(path).exists() {
            println!("cargo:rustc-link-search=native={path}");
        }
    }
    println!("cargo:rustc-link-lib=mpv");
}
