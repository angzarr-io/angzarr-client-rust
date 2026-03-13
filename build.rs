fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read version from VERSION file and inject as environment variable
    let version = std::fs::read_to_string("VERSION")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=ANGZARR_CLIENT_VERSION={}", version);
    println!("cargo:rerun-if-changed=VERSION");

    // Proto generation is opt-in via ANGZARR_PROTO_ROOT env var
    // Pre-generated code in src/proto/generated.rs is used by default
    // To regenerate: ANGZARR_PROTO_ROOT=path/to/protos cargo build
    // Then copy target/debug/build/angzarr-client-*/out/angzarr.rs to src/proto/generated.rs
    if let Ok(proto_root) = std::env::var("ANGZARR_PROTO_ROOT") {
        println!("cargo:rerun-if-changed={}", proto_root);

        let protos: Vec<String> = vec![
            format!("{}/angzarr/types.proto", proto_root),
            format!("{}/angzarr/command_handler.proto", proto_root),
            format!("{}/angzarr/projector.proto", proto_root),
            format!("{}/angzarr/saga.proto", proto_root),
            format!("{}/angzarr/process_manager.proto", proto_root),
            format!("{}/angzarr/query.proto", proto_root),
            format!("{}/angzarr/stream.proto", proto_root),
            format!("{}/angzarr/upcaster.proto", proto_root),
            format!("{}/angzarr/meta.proto", proto_root),
            format!("{}/angzarr/cloudevents.proto", proto_root),
        ];

        // Enable prost::Name trait for type reflection
        let mut prost_config = prost_build::Config::new();
        prost_config.enable_type_names();

        tonic_build::configure()
            .build_server(true)
            .build_client(true)
            .type_attribute(
                ".angzarr.BusinessResponse.result",
                "#[allow(clippy::large_enum_variant)]",
            )
            .compile_protos_with_config(prost_config, &protos, &[proto_root])?;
    }

    Ok(())
}
