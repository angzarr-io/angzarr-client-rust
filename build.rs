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
        "angzarr-project/proto/io/angzarr/v1/types.proto",
        "angzarr-project/proto/io/angzarr/v1/command_handler.proto",
        "angzarr-project/proto/io/angzarr/v1/projector.proto",
        "angzarr-project/proto/io/angzarr/v1/saga.proto",
        "angzarr-project/proto/io/angzarr/v1/process_manager.proto",
        "angzarr-project/proto/io/angzarr/v1/query.proto",
        "angzarr-project/proto/io/angzarr/v1/stream.proto",
        "angzarr-project/proto/io/angzarr/v1/upcaster.proto",
        "angzarr-project/proto/io/angzarr/v1/meta.proto",
        "angzarr-project/proto/io/angzarr/v1/cloudevents.proto",
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

    // Framework messages are the angzarr-router crate's types, so the
    // router's dispatch tables and this crate's gRPC services and clients
    // share one set of Rust types. Only messages the router does not
    // generate (query, projector, stream, meta, cloudevents) and the
    // service stubs are emitted here.
    let mut builder = tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir(&out_dir)
        .extern_path(".sererr.v1", "::angzarr_router::proto::sererr::v1");
    for file in ROUTER_PROTO_FILES {
        let text = std::fs::read_to_string(file)?;
        for name in top_level_types(&text) {
            builder = builder.extern_path(
                format!(".io.angzarr.v1.{name}"),
                format!("::angzarr_router::pb::{name}"),
            );
        }
    }
    builder.compile_with_config(prost_config, &proto_files, &["angzarr-project/proto"])?;
    Ok(())
}

/// The proto files whose messages the angzarr-router crate generates.
const ROUTER_PROTO_FILES: &[&str] = &[
    "angzarr-project/proto/io/angzarr/v1/types.proto",
    "angzarr-project/proto/io/angzarr/v1/command_handler.proto",
    "angzarr-project/proto/io/angzarr/v1/saga.proto",
    "angzarr-project/proto/io/angzarr/v1/process_manager.proto",
    "angzarr-project/proto/io/angzarr/v1/upcaster.proto",
];

/// Names of the messages and enums declared at the top level of a proto
/// file (nested types travel with their parent's extern path).
fn top_level_types(proto: &str) -> Vec<String> {
    proto
        .lines()
        .filter_map(|line| {
            let rest = line
                .strip_prefix("message ")
                .or_else(|| line.strip_prefix("enum "))?;
            Some(
                rest.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()?
                    .to_string(),
            )
        })
        .filter(|name| !name.is_empty())
        .collect()
}
