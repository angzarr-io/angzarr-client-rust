fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Version injection
    let version = std::fs::read_to_string("VERSION")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=ANGZARR_CLIENT_VERSION={}", version);
    println!("cargo:rerun-if-changed=VERSION");

    // Proto generation from angzarr-project submodule
    let proto_files = [
        "angzarr-project/proto/angzarr_client/proto/angzarr/types.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/command_handler.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/projector.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/saga.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/process_manager.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/query.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/stream.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/upcaster.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/meta.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/cloudevents.proto",
    ];
    for file in &proto_files {
        println!("cargo:rerun-if-changed={}", file);
    }

    let mut prost_config = prost_build::Config::new();
    prost_config.enable_type_names();

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .type_attribute(
            ".angzarr_client.proto.angzarr.BusinessResponse.result",
            "#[allow(clippy::large_enum_variant)]",
        )
        .compile_with_config(
            prost_config,
            &proto_files,
            &["angzarr-project/proto"],
        )?;
    Ok(())
}
