fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let plist = manifest.join("../../packaging/macos/Info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Give AppKit bundle metadata even when launched with `cargo run`.
        // Restrict this to the app binary, not tests or benchmark executables.
        println!(
            "cargo:rustc-link-arg-bin=inkstone=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
}
