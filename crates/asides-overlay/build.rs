//! Embeds the Help page's clips (`help/`, made by `scripts/make-help.py`; Windows', made on Windows,
//! in `help-windows/`): each file by its name, for the window's assets (`assets.rs`). On Windows,
//! also the icon (`packaging/asides.ico`) and version information: GPUI gives its windows the
//! program's icon (the asides window's taskbar button).

use std::path::Path;

fn main() {
    let crate_dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).to_path_buf();
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let dir = match windows && crate_dir.join("help-windows").is_dir() {
        true => crate_dir.join("help-windows"),
        false => crate_dir.join("help"),
    };
    println!("cargo:rerun-if-changed={}", crate_dir.join("help").display());
    println!("cargo:rerun-if-changed={}", crate_dir.join("help-windows").display());
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    files.sort();
    let mut out = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy();
        out += &format!("    ({name:?}, include_bytes!({:?})),\n", path.display().to_string());
    }
    out += "];\n";
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("help.rs"), out).unwrap();

    println!("cargo:rerun-if-changed=../../packaging/asides.ico");
    if windows {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../packaging/asides.ico");
        resource.set("ProductName", "asides");
        resource.set("FileDescription", "asides");
        if let Err(err) = resource.compile() {
            println!("cargo:warning=asides-overlay.exe without its icon: {err}");
        }
    }
}
