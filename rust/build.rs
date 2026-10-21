//! Build script:
//! - embeds the application icon / version info into the Windows executable
//! - copies the WinDivert DLL (and driver .sys files) next to the built binary
//!   so `cargo run` just works. WinDivert.dll is loaded dynamically at runtime.

use std::env;
use std::path::Path;
use std::path::PathBuf;

const WINDIVERT_DIR: &str = "../external/WinDivert-2.2.0-C";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/clumsy-icon.ico");

    embed_icon();

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let out_dir = env::var("OUT_DIR").unwrap();
    // target dir = OUT_DIR/../../../ (target/<profile>)
    let target_dir = Path::new(&out_dir)
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .expect("could not resolve target dir");

    let arch_dir = match target_arch.as_str() {
        "x86" => "x86",
        _ => "x64",
    };
    copy_windivert(Path::new(WINDIVERT_DIR).join(arch_dir), &target_dir);

    let config_src = Path::new("../etc/config.txt");
    let config_dst = target_dir.join("config.txt");
    if config_src.exists() && need_copy(config_src, &config_dst) {
        let _ = std::fs::copy(config_src, &config_dst);
    }
}

fn embed_icon() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if let Err(e) = winresource::WindowsResource::new()
            .set_icon("assets/clumsy-icon.ico")
            .set("FileDescription", "ClumsyMagi - network condition simulator")
            .set("ProductName", "ClumsyMagi")
            .compile()
        {
            // Non-fatal: a missing icon should not break the build.
            eprintln!("cargo:warning=failed to embed icon resource: {e}");
        }
    }
}

fn copy_windivert(src_dir: PathBuf, target_dir: &Path) {
    let files = ["WinDivert.dll", "WinDivert.sys", "WinDivert64.sys", "WinDivert32.sys"];
    for name in files {
        let src = src_dir.join(name);
        if !src.exists() {
            eprintln!("cargo:warning=WinDivert file missing: {}", src.display());
            continue;
        }
        let dst = target_dir.join(name);
        if need_copy(&src, &dst) {
            if let Err(e) = std::fs::copy(&src, &dst) {
                // Can legitimately fail when the app is running (dll locked); warn only.
                eprintln!("cargo:warning=could not copy {}: {e}", src.display());
            }
        }
    }
}

fn need_copy(src: &Path, dst: &Path) -> bool {
    match (std::fs::metadata(src), std::fs::metadata(dst)) {
        (Ok(s), Ok(d)) => s.len() != d.len() || s.modified().ok() > d.modified().ok(),
        (Ok(_), Err(_)) => true,
        _ => false,
    }
}
