//! Compiles the Windows icon resource (ui/win/wisp.rc) into the exe when building for
//! Windows; nothing to do for the Mac.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=ui/win/wisp.rc");
    println!("cargo:rerun-if-changed=ui/win/wisp.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("wisp-res.o");
    let status = Command::new("x86_64-w64-mingw32-windres")
        .args(["--include-dir", "ui/win", "ui/win/wisp.rc", "-O", "coff", "-o"])
        .arg(&out)
        .status()
        .expect("x86_64-w64-mingw32-windres (brew install mingw-w64)");
    assert!(status.success(), "windres failed");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
