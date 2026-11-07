//! Windows builds get the BaseLens icon and version info inside the .exe.
//! Nothing happens on Linux or macOS.

fn main() {
    println!("cargo:rerun-if-changed=assets/baselens.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/baselens.ico");
        res.set("ProductName", "BaseLens");
        res.set("FileDescription", "BaseLens — database editor");
        res.set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            println!("cargo:warning=the icon was not embedded in baselens.exe: {e}");
        }
    }
}
