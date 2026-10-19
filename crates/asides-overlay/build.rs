//! Embeds the Help page's clips (`help/`, made by `scripts/make-help.py`): each file by its name, for
//! the window's assets (`assets.rs`).

use std::path::Path;

fn main() {
    let dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("help");
    println!("cargo:rerun-if-changed={}", dir.display());
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
}
