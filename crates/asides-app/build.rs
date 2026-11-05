//! On Windows: the icon (`packaging/asides.ico`, made by `scripts/make-icon.swift`) and version
//! information in asides-app.exe itself: Explorer, the Start menu and Settings' startup apps show
//! them, and Task Manager names the process by its description.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/asides.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../packaging/asides.ico");
    resource.set("ProductName", "asides");
    resource.set("FileDescription", "asides");
    if let Err(err) = resource.compile() {
        println!("cargo:warning=asides-app.exe without its icon: {err}");
    }
}
