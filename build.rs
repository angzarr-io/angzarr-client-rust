fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Version injection (always runs)
    let version = std::fs::read_to_string("VERSION")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=ANGZARR_CLIENT_VERSION={}", version);
    println!("cargo:rerun-if-changed=VERSION");
    println!("cargo:rerun-if-env-changed=GENERATE_PROTOS");

    // Proto generation moved out of build.rs per project_proto_generation_model
    // (cross-language pattern: pre-build trigger lives in `just generate-proto`,
    // never in build-tool integration). build.rs only emits the bindings when
    // explicitly opted in via `GENERATE_PROTOS=1` (the `just generate-proto`
    // recipe sets this when it needs to refresh the source-tree bindings).
    //
    // Normal `cargo build` paths skip codegen and consume the pre-emitted
    // `src/proto/*.rs` files via `include!` in `src/proto.rs`. Those files are
    // gitignored; lefthook fires `just generate-proto` on post-checkout /
    // post-merge so fresh clones get them automatically.
    if std::env::var("GENERATE_PROTOS").is_err() {
        return Ok(());
    }

    let proto_files = [
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/types.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/command_handler.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/projector.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/saga.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/process_manager.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/query.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/stream.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/upcaster.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/meta.proto",
        "angzarr-project/proto/angzarr_client/proto/angzarr/v1/cloudevents.proto",
    ];
    for file in &proto_files {
        println!("cargo:rerun-if-changed={}", file);
    }

    let out_dir = std::path::PathBuf::from(
        std::env::var("ANGZARR_PROTO_OUT_DIR").unwrap_or_else(|_| "src/proto".to_string()),
    );
    std::fs::create_dir_all(&out_dir)?;

    let mut prost_config = prost_build::Config::new();
    prost_config.enable_type_names();

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir(&out_dir)
        .type_attribute(
            ".angzarr_client.proto.angzarr.BusinessResponse.result",
            "#[allow(clippy::large_enum_variant)]",
        )
        .compile_with_config(prost_config, &proto_files, &["angzarr-project/proto"])?;
    Ok(())
}
