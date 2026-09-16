fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");

    // winresource derives FileVersion and ProductVersion from Cargo's package
    // metadata, so the release version has a single source of truth in Cargo.toml.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set("ProductName", "Epub3-Kindle")
            .set("FileDescription", env!("CARGO_PKG_DESCRIPTION"))
            .set("OriginalFilename", "epub3-kindle.exe")
            .compile()
            .expect("failed to compile Windows version resource");
    }
}
