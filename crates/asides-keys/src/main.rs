//! `asides-keys`: types into a terminal on Linux (`linux.rs`). Elsewhere there's nothing for it to do,
//! but it's in the workspace, so it builds.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
use linux::main;

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("asides-keys is for Linux");
    std::process::exit(1);
}
