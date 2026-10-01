//! Step definitions for parity/client/parity.feature.
//!
//! Presence is checked against [`EXPORTED`] / [`TESTING_EXPORTED`], which
//! the `compile_probe` module pins: dropping a listed re-export stops this
//! file compiling. Absence from the crate root is checked by probe modules
//! that glob-import the root next to stand-ins of the same names — a name
//! the root also exports is ambiguous at its use site and fails the build.

#![allow(dead_code, unused_imports)]

use std::path::Path;
use std::sync::Arc;

use angzarr_client::proto::command_handler_service_server::CommandHandlerServiceServer;
use angzarr_client::{ComponentHost, ServerConfig};
use cucumber::{given, then, World};
use tonic::server::NamedService;

use crate::common::host_fixtures::{
    connect, Gate, OrderComponent, OrderReportService, ORDER_REPORT_SERVICE,
};

#[derive(Debug, Default, World)]
pub struct ParityWorld {
    importable: bool,
}

const EXPORTED: &[&str] = &[
    // Clients
    "CommandHandlerClient",
    "QueryClient",
    "SpeculativeClient",
    "DomainClient",
    // Errors
    "ClientError",
    "CommandRejectedError",
    // Constants
    "TYPE_URL_PREFIX",
    "UNKNOWN_DOMAIN",
    "WILDCARD_DOMAIN",
    "DEFAULT_EDITION",
    "META_ANGZARR_DOMAIN",
    "PROJECTION_DOMAIN_PREFIX",
    "PROJECTION_TYPE_URL",
    // Identity
    "compute_root",
    "to_proto_bytes",
    // Retry
    "RetryPolicy",
    "ExponentialBackoffRetry",
    "default_retry_policy",
    // Builders
    "CommandBuilder",
    "QueryBuilder",
    // Component host
    "ComponentHost",
    "configure_logging",
    "get_transport_config",
];

/// Names reachable from `angzarr_client::testing` (the `testing` feature,
/// which this crate's dev-dependency enables) and not from the root.
const TESTING_EXPORTED: &[&str] = &[
    "make_timestamp",
    "make_cover",
    "make_event_page",
    "make_event_book",
    "make_command_page",
    "make_command_book",
    "uuid_for",
    "uuid_str_for",
    "uuid_obj_for",
    "DEFAULT_TEST_NAMESPACE",
    "ScenarioContext",
];

/// Predicates implemented on `ClientError`.
const ERROR_PREDICATES_IMPLEMENTED: &[&str] = &[
    "is_not_found",
    "is_precondition_failed",
    "is_invalid_argument",
    "is_connection_error",
];

/// Example and business concepts the client must not name: the generic
/// client-tier fixture vocabulary and the example applications' domains.
const BUSINESS_CONCEPTS: &[&str] = &[
    "order",
    "payment",
    "inventory",
    "shipping",
    "shipment",
    "fulfillment",
    "cart",
    "customer",
    "product",
    "stock",
    "reservation",
    "player",
    "table",
    "hand",
    "tournament",
    "poker",
    "blackjack",
    "card",
    "deck",
    "chip",
];

fn check(name: &str) {
    assert!(
        EXPORTED.contains(&name),
        "\"{name}\" is not re-exported from angzarr_client"
    );
}

fn check_testing(name: &str) {
    assert!(
        TESTING_EXPORTED.contains(&name),
        "\"{name}\" is not exported from angzarr_client::testing"
    );
}

// --- Background ------------------------------------------------------------

#[given("the angzarr client library is importable at its public root")]
async fn given_library_importable(world: &mut ParityWorld) {
    world.importable = true;
}

// --- Generic symbol checks -------------------------------------------------

#[then(expr = "the {string} symbol is exported")]
async fn then_symbol_exported(_world: &mut ParityWorld, name: String) {
    check(&name);
}

#[then(expr = "the {string} constant is exported")]
async fn then_constant_exported(_world: &mut ParityWorld, name: String) {
    check(&name);
}

#[then(expr = "the {string} symbol is exported from the testing module")]
async fn then_symbol_in_testing(_world: &mut ParityWorld, name: String) {
    check_testing(&name);
}

#[then(expr = "the {string} constant is exported from the testing module")]
async fn then_constant_in_testing(_world: &mut ParityWorld, name: String) {
    check_testing(&name);
}

#[then("none of the testing helpers is exported from the client's root")]
async fn then_testing_not_at_root(_world: &mut ParityWorld) {
    for name in TESTING_EXPORTED {
        assert!(
            !EXPORTED.contains(name),
            "\"{name}\" is re-exported from the angzarr_client root"
        );
    }
    testing_root_probe::check();
}

#[then(expr = "the {string} constant is exported with value {string}")]
async fn then_constant_value(_world: &mut ParityWorld, name: String, value: String) {
    check(&name);
    let actual = match name.as_str() {
        "TYPE_URL_PREFIX" => angzarr_client::TYPE_URL_PREFIX,
        other => panic!("no value probe for constant {other}"),
    };
    assert_eq!(actual, value, "{name} value");
}

#[then(expr = "the client exposes the {string} error predicate")]
async fn then_error_predicate_exposed(_world: &mut ParityWorld, name: String) {
    assert!(
        ERROR_PREDICATES_IMPLEMENTED.contains(&name.as_str()),
        "error predicate \"{name}\" is not implemented on ClientError"
    );
}

// --- Router binding ---------------------------------------------------------

#[then("the router binding is exported from the router module")]
async fn then_router_binding(_world: &mut ParityWorld) {
    let engine = std::any::type_name::<angzarr_client::router::binding::aggregate::FactRecord>();
    assert!(
        engine.starts_with("angzarr_router::"),
        "angzarr_client::router::binding is {engine}, not angzarr-router"
    );
}

#[then(
    "the client's root exports no dispatch-engine API: no handler decorators, Router builders, handler gRPC adapters or compensation helpers"
)]
async fn then_no_dispatch_at_root(_world: &mut ParityWorld) {
    dispatch_root_probe::check();
}

// --- Business concepts -------------------------------------------------------

/// Public item, module and gRPC service names declared under `dir`.
fn public_names(dir: &Path, out: &mut Vec<(String, String)>) {
    let decl = regex_lite_public_decl();
    for entry in std::fs::read_dir(dir).expect("source directory") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            public_names(&path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("source file");
        for line in text.lines() {
            if let Some(name) = decl(line) {
                out.push((name, path.display().to_string()));
            }
        }
    }
}

/// Extract the declared name from a `pub fn|struct|enum|trait|const|static|
/// mod|type` line, or the name string of a generated message or gRPC
/// service.
fn regex_lite_public_decl() -> impl Fn(&str) -> Option<String> {
    |line: &str| {
        let t = line.trim_start();
        for prefix in [
            "const NAME: &'static str = \"",
            "pub const SERVICE_NAME: &str = \"",
        ] {
            if let Some(rest) = t.strip_prefix(prefix) {
                return rest.split('"').next().map(|s| s.to_string());
            }
        }
        let rest = t.strip_prefix("pub ")?;
        let rest = rest.strip_prefix("async ").unwrap_or(rest);
        let rest = rest.strip_prefix("const fn ").or_else(|| {
            [
                "fn ", "struct ", "enum ", "trait ", "const ", "static ", "mod ", "type ",
            ]
            .iter()
            .find_map(|k| rest.strip_prefix(k))
        })?;
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some(name)
    }
}

/// Lower-case words of an identifier or dotted service name.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        current.push(c.to_ascii_lowercase());
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[then(
    "no exported symbol, module or gRPC service of the client names an example or business concept"
)]
async fn then_no_business_names(_world: &mut ParityWorld) {
    let mut names = Vec::new();
    public_names(Path::new("src"), &mut names);
    public_names(Path::new("angzarr-macros/src"), &mut names);
    assert!(
        names.iter().any(|(n, _)| n == "ComponentHost"),
        "the scan sees the crate's public items"
    );
    assert!(
        names
            .iter()
            .any(|(n, _)| n == "io.angzarr.v1.CommandHandlerService"),
        "the scan sees the crate's gRPC services"
    );
    let offending: Vec<_> = names
        .iter()
        .filter(|(n, _)| {
            words(n)
                .iter()
                .any(|w| BUSINESS_CONCEPTS.contains(&w.as_str()))
        })
        .collect();
    assert!(
        offending.is_empty(),
        "business concepts named: {offending:?}"
    );
}

#[then(
    "the component host serves only framework services and the services an application registers"
)]
async fn then_host_serves_only(_world: &mut ParityWorld) {
    let ch = <CommandHandlerServiceServer<angzarr_client::handler::CommandHandlerGrpc> as NamedService>::NAME;
    let gate = Arc::new(Gate::default());
    let running = ComponentHost::new()
        .with_handler(move || OrderComponent(Arc::clone(&gate)))
        .with_service(OrderReportService)
        .with_transport(ServerConfig {
            port: 0,
            uds_path: None,
        })
        .start()
        .await
        .expect("host starts");
    assert_eq!(
        running.services(),
        &[
            angzarr_client::host::HEALTH_SERVICE_NAME.to_string(),
            ch.to_string(),
            ORDER_REPORT_SERVICE.to_string(),
        ]
    );
    // A framework service with no registered component is not served.
    let channel = connect(running.address()).await;
    let status = angzarr_client::proto::saga_service_client::SagaServiceClient::new(channel)
        .handle(angzarr_client::proto::SagaHandleRequest::default())
        .await
        .expect_err("no saga is registered");
    assert_eq!(status.code(), tonic::Code::Unimplemented);
    running.shutdown().await.expect("clean shutdown");
}

// --- Compile-time probes -----------------------------------------------------

/// Referencing each listed name keeps it reachable where the scenarios say.
mod compile_probe {
    #![allow(unused_imports, dead_code)]
    use angzarr_client::router::binding;
    use angzarr_client::testing::{
        make_command_book, make_command_page, make_cover, make_event_book, make_event_page,
        make_timestamp, uuid_for, uuid_obj_for, uuid_str_for, ScenarioContext,
        DEFAULT_TEST_NAMESPACE,
    };
    use angzarr_client::{
        compute_root, configure_logging, default_retry_policy, get_transport_config,
        to_proto_bytes, ClientError, CommandBuilder, CommandHandlerClient, CommandRejectedError,
        ComponentHost, DomainClient, ExponentialBackoffRetry, QueryBuilder, QueryClient,
        RetryPolicy, SpeculativeClient, DEFAULT_EDITION, META_ANGZARR_DOMAIN,
        PROJECTION_DOMAIN_PREFIX, PROJECTION_TYPE_URL, TYPE_URL_PREFIX, UNKNOWN_DOMAIN,
        WILDCARD_DOMAIN,
    };
}

/// Compiles only while the crate root exports none of the testing helpers.
#[allow(non_upper_case_globals, dead_code)]
mod testing_root_probe {
    mod fallback {
        pub const make_timestamp: u8 = 0;
        pub const make_cover: u8 = 0;
        pub const make_event_page: u8 = 0;
        pub const make_event_book: u8 = 0;
        pub const make_command_page: u8 = 0;
        pub const make_command_book: u8 = 0;
        pub const uuid_for: u8 = 0;
        pub const uuid_str_for: u8 = 0;
        pub const uuid_obj_for: u8 = 0;
        pub const DEFAULT_TEST_NAMESPACE: u8 = 0;
        pub struct ScenarioContext;
    }

    #[allow(unused_imports)]
    use angzarr_client::*;
    use fallback::*;

    pub fn check() {
        let names: [u8; 10] = [
            make_timestamp,
            make_cover,
            make_event_page,
            make_event_book,
            make_command_page,
            make_command_book,
            uuid_for,
            uuid_str_for,
            uuid_obj_for,
            DEFAULT_TEST_NAMESPACE,
        ];
        assert_eq!(names, [0; 10]);
        let _: Option<ScenarioContext> = None::<fallback::ScenarioContext>;
    }
}

/// Compiles only while the crate root exports no dispatch-engine API:
/// handler kind attributes and markers (macro namespace), Router builders
/// and runtime routers, handler gRPC adapters and compensation helpers.
#[allow(non_upper_case_globals, non_camel_case_types, dead_code, unused_macros)]
mod dispatch_root_probe {
    mod fallback {
        macro_rules! command_handler {
            () => {
                0u8
            };
        }
        macro_rules! saga {
            () => {
                0u8
            };
        }
        macro_rules! process_manager {
            () => {
                0u8
            };
        }
        macro_rules! projector {
            () => {
                0u8
            };
        }
        macro_rules! upcaster {
            () => {
                0u8
            };
        }
        macro_rules! handles {
            () => {
                0u8
            };
        }
        macro_rules! handles_fact {
            () => {
                0u8
            };
        }
        macro_rules! applies {
            () => {
                0u8
            };
        }
        macro_rules! rejected {
            () => {
                0u8
            };
        }
        macro_rules! state_factory {
            () => {
                0u8
            };
        }
        macro_rules! upcasts {
            () => {
                0u8
            };
        }
        pub(crate) use {
            applies, command_handler, handles, handles_fact, process_manager, projector, rejected,
            saga, state_factory, upcaster, upcasts,
        };

        pub struct Router;
        pub struct Built;
        pub struct CommandHandlerRouter;
        pub struct SagaRouter;
        pub struct ProcessManagerRouter;
        pub struct ProjectorRouter;
        pub struct UpcasterRouter;
        pub struct CommandHandlerGrpc;
        pub struct SagaGrpc;
        pub struct ProcessManagerGrpc;
        pub struct ProjectorGrpc;
        pub struct UpcasterGrpc;
        pub struct CompensationContext;
        pub const delegate_to_framework: u8 = 0;
        pub const emit_compensation_events: u8 = 0;
        pub const pm_delegate_to_framework: u8 = 0;
        pub const pm_emit_compensation_events: u8 = 0;
    }

    #[allow(unused_imports)]
    use angzarr_client::*;
    use fallback::*;

    pub fn check() {
        let markers: [u8; 11] = [
            command_handler!(),
            saga!(),
            process_manager!(),
            projector!(),
            upcaster!(),
            handles!(),
            handles_fact!(),
            applies!(),
            rejected!(),
            state_factory!(),
            upcasts!(),
        ];
        assert_eq!(markers, [0; 11]);
        let helpers: [u8; 4] = [
            delegate_to_framework,
            emit_compensation_events,
            pm_delegate_to_framework,
            pm_emit_compensation_events,
        ];
        assert_eq!(helpers, [0; 4]);
        let _: Option<Router> = None::<fallback::Router>;
        let _: Option<Built> = None::<fallback::Built>;
        let _: Option<CommandHandlerRouter> = None::<fallback::CommandHandlerRouter>;
        let _: Option<SagaRouter> = None::<fallback::SagaRouter>;
        let _: Option<ProcessManagerRouter> = None::<fallback::ProcessManagerRouter>;
        let _: Option<ProjectorRouter> = None::<fallback::ProjectorRouter>;
        let _: Option<UpcasterRouter> = None::<fallback::UpcasterRouter>;
        let _: Option<CommandHandlerGrpc> = None::<fallback::CommandHandlerGrpc>;
        let _: Option<SagaGrpc> = None::<fallback::SagaGrpc>;
        let _: Option<ProcessManagerGrpc> = None::<fallback::ProcessManagerGrpc>;
        let _: Option<ProjectorGrpc> = None::<fallback::ProjectorGrpc>;
        let _: Option<UpcasterGrpc> = None::<fallback::UpcasterGrpc>;
        let _: Option<CompensationContext> = None::<fallback::CompensationContext>;
    }
}
