fn main() {
    println!("cargo:rerun-if-changed=packaging/macos/Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        // Give AppKit bundle metadata even when launched with `cargo run`.
        // Restrict this to the app binary, not tests or benchmark executables.
        println!(
            "cargo:rustc-link-arg-bin=inkstone=-Wl,-sectcreate,__TEXT,__info_plist,{manifest}/packaging/macos/Info.plist"
        );
    }
}
