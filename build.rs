fn main() {
    // Read version from VERSION file and inject as environment variable
    let version = std::fs::read_to_string("VERSION")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=ANGZARR_CLIENT_VERSION={}", version);
    println!("cargo:rerun-if-changed=VERSION");

}
