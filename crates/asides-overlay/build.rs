//! On Windows, the icon (`packaging/asides.ico`) and version information: GPUI gives its windows
//! the program's icon (the asides window's taskbar button).

fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows";
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
