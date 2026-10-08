fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let resources = manifest.join("assets");
        let rc = resources.join("inkstone.rc");
        println!("cargo:rerun-if-changed={}", rc.display());
        println!(
            "cargo:rerun-if-changed={}",
            resources.join("inkstone.ico").display()
        );
        embed_resource::compile_for(
            &rc,
            ["inkstone"],
            embed_resource::ParamsIncludeDirs([resources]),
        )
        .manifest_required()
        .expect("compile Windows application icon (Windows SDK required)");
    }
    let plist = manifest.join("../../packaging/macos/Info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let generated =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("Info.plist");
        let template = std::fs::read_to_string(&plist).expect("read macOS plist template");
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        std::fs::write(&generated, template.replace("@VERSION@", &version))
            .expect("write versioned macOS plist");
        // Give AppKit bundle metadata even when launched with `cargo run`.
        // Restrict this to the app binary, not tests or benchmark executables.
        println!(
            "cargo:rustc-link-arg-bin=inkstone=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            generated.display()
        );
    }
}
