fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows.manifest");
        println!("cargo:rerun-if-changed=windows.manifest");
        println!("cargo:rustc-link-arg-bin=notepad-md=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=notepad-md=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
