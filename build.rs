//! Windows builds get the BaseLens icon, version info and an application
//! manifest (high-DPI, modern controls, UTF-8, no admin prompt) inside the .exe.
//! Nothing happens on Linux or macOS.

fn main() {
    println!("cargo:rerun-if-changed=assets/baselens.ico");
    println!("cargo:rerun-if-changed=packaging/windows/baselens.exe.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/baselens.ico");
        res.set_manifest_file("packaging/windows/baselens.exe.manifest");
        res.set("ProductName", "BaseLens");
        res.set("FileDescription", "BaseLens — database editor");
        res.set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            println!("cargo:warning=the icon was not embedded in baselens.exe: {e}");
        }
    }
}
