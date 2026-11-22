fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/pane-md.rc");
        println!("cargo:rerun-if-changed=assets/pane-md.ico");
        embed_resource::compile_for("assets/pane-md.rc", ["pane-md"], embed_resource::NONE)
            .manifest_optional()
            .expect("embed the Windows app icon");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let directory = std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory");
        let manifest = std::path::PathBuf::from(directory).join("windows.manifest");
        println!("cargo:rerun-if-changed=windows.manifest");
        println!("cargo:rustc-link-arg-bin=pane-md=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=pane-md=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
