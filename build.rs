fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/pane-md.rc");
        println!("cargo:rerun-if-changed=assets/pane-md.ico");
        let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
        let icon = manifest
            .join("assets/pane-md.ico")
            .display()
            .to_string()
            .replace('\\', "/");
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let numbers = ["MAJOR", "MINOR", "PATCH"]
            .map(|part| std::env::var(format!("CARGO_PKG_VERSION_{part}")).unwrap());
        let file_version = format!("{},{},{},0", numbers[0], numbers[1], numbers[2]);
        let resource = include_str!("assets/pane-md.rc")
            .replace("@ICON@", &icon)
            .replace("@FILEVERSION@", &file_version)
            .replace("@VERSION@", &version);
        let path =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("pane-md.rc");
        std::fs::write(&path, resource).expect("write Windows version resource");
        embed_resource::compile_for(path, ["pane-md", "pane-md-editor"], embed_resource::NONE)
            .manifest_optional()
            .expect("embed the Windows app icon");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let directory = std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory");
        let manifest = std::path::PathBuf::from(directory).join("assets/windows.manifest");
        println!("cargo:rerun-if-changed=assets/windows.manifest");
        println!("cargo:rustc-link-arg-bin=pane-md=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=pane-md=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
