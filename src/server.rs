//! Transport configuration and runners for hosting components.
//!
//! [`get_transport_config`] resolves the transport (TCP or a Unix socket)
//! from the environment the coordinator sets. Each `run_*_server` function
//! serves one built router through a [`crate::host::ComponentHost`] until
//! SIGINT / SIGTERM; use the host directly to serve several components or
//! application services together.

use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use tracing::warn;

use crate::error::{ClientError, Result};
use crate::error_codes::{codes, keys, messages};
use crate::host::ComponentHost;
use crate::router::routers::{CommandHandlerRouter, ProcessManagerRouter, SagaRouter};
use crate::router::Built;

/// Initialize a JSON tracing subscriber filtered by `RUST_LOG` (default `info`).
///
/// Idempotent — `try_init` swallows the "already set" error so a second call
/// in the same process is a no-op.
pub fn configure_logging() {
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}

/// Configuration for a gRPC runner.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// TCP port (used when `uds_path` is `None`).
    pub port: u16,
    /// Unix domain socket path. When `Some`, supersedes `port`.
    pub uds_path: Option<PathBuf>,
}

impl ServerConfig {
    /// Resolve from env, matching the variables the coordinator sets when
    /// it spawns a component (`TRANSPORT_TYPE`, `UDS_BASE_PATH`,
    /// `SERVICE_NAME`, `DOMAIN`) and Python's runner:
    ///
    /// - `TRANSPORT_TYPE=uds` (case-insensitive): UDS at
    ///   `{UDS_BASE_PATH:-/tmp/angzarr}/{SERVICE_NAME:-business}-{qualifier}.sock`,
    ///   where the qualifier is the first set of `DOMAIN`, `SAGA_NAME`,
    ///   `PROJECTOR_NAME`; with no qualifier the file is `{service}.sock`.
    /// - `TRANSPORT_TYPE` set to anything else: TCP.
    /// - `TRANSPORT_TYPE` unset: UDS when `UDS_BASE_PATH`, `SERVICE_NAME` and
    ///   `DOMAIN` are all set, otherwise TCP.
    ///
    /// TCP reads the port from `PORT` or `GRPC_PORT`, falling back to
    /// `default_port`.
    ///
    /// Server-side reads `UDS_BASE_PATH` while the client-side
    /// [`crate::transport::resolve_ch_endpoint`] reads `ANGZARR_UDS_BASE`;
    /// every language client uses that same split.
    ///
    /// This function is **pure** — no filesystem side effects. The runner is
    /// responsible for creating the parent directory and removing any stale
    /// socket file at the chosen path.
    pub fn from_env(default_port: u16) -> Self {
        let set = |k: &str| env::var(k).ok().filter(|v| !v.is_empty());
        let uds_path = match set("TRANSPORT_TYPE") {
            Some(t) if t.eq_ignore_ascii_case("uds") => {
                let base = set("UDS_BASE_PATH").unwrap_or_else(|| "/tmp/angzarr".into());
                let service = set("SERVICE_NAME").unwrap_or_else(|| "business".into());
                let qualifier = set("DOMAIN")
                    .or_else(|| set("SAGA_NAME"))
                    .or_else(|| set("PROJECTOR_NAME"));
                let file = match qualifier {
                    Some(q) => format!("{service}-{q}.sock"),
                    None => format!("{service}.sock"),
                };
                Some(PathBuf::from(base).join(file))
            }
            Some(_) => None,
            None => match (set("UDS_BASE_PATH"), set("SERVICE_NAME"), set("DOMAIN")) {
                (Some(base), Some(service), Some(domain)) => {
                    Some(PathBuf::from(base).join(format!("{service}-{domain}.sock")))
                }
                _ => None,
            },
        };
        let port = env::var("PORT")
            .or_else(|_| env::var("GRPC_PORT"))
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default_port);
        Self { port, uds_path }
    }
}

/// Resolve transport configuration from environment. Single canonical entry
/// point — the only env reader callers should use.
pub fn get_transport_config(default_port: u16) -> ServerConfig {
    ServerConfig::from_env(default_port)
}

/// Env var name for the full TCP bind address override (`host:port`).
///
/// Audit #77: when set, supersedes the default `[::]:{port}` composition
/// and the `PORT` / `GRPC_PORT` resolution. IPv6 hosts must include
/// brackets (e.g. `[::1]:50052`); IPv4 hosts are written bare.
pub const ENV_BIND_ADDRESS: &str = "ANGZARR_BIND_ADDRESS";

/// Default TCP bind host. `"[::]"` is the IPv6 wildcard, which on
/// Linux (`IPV6_V6ONLY=0` by default) accepts both IPv4 (via IPv4-mapped
/// IPv6) and IPv6 connections — matching Python's posture per audit #77.
pub const DEFAULT_BIND_HOST: &str = "[::]";

/// Compute the TCP bind address.
///
/// Returns `ANGZARR_BIND_ADDRESS` verbatim when set, otherwise composes
/// `[::]:{default_port}`. Pure read of env state; intended to be called
/// once per server start, immediately before `parse::<SocketAddr>()`.
pub fn resolve_bind_address(default_port: u16) -> String {
    env::var(ENV_BIND_ADDRESS).unwrap_or_else(|_| format!("{}:{}", DEFAULT_BIND_HOST, default_port))
}

/// Serve one built router of any kind until SIGINT / SIGTERM.
pub async fn run_server(default_port: u16, built: Built) -> Result<()> {
    ComponentHost::new()
        .with_router(built)
        .with_default_port(default_port)
        .serve()
        .await
}

/// Remove a stale UDS socket file at `path`. No-op if the path does not exist.
pub fn cleanup_socket(path: impl AsRef<Path>) {
    let p = path.as_ref();
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
}

/// Parse a `host:port` string into a `SocketAddr`, surfacing a structured
/// `INVALID_BIND_ADDRESS` error instead of panicking on operator typos.
pub(crate) fn parse_bind_address(addr_str: &str) -> Result<SocketAddr> {
    addr_str.parse::<SocketAddr>().map_err(|e| {
        ClientError::invalid_argument(
            codes::INVALID_BIND_ADDRESS,
            messages::INVALID_BIND_ADDRESS,
            [
                (keys::INPUT, addr_str.to_string()),
                (keys::CAUSE, e.to_string()),
            ],
        )
    })
}

/// Ensure the parent directory exists for a UDS path, surfacing a
/// structured error if the create fails (read-only fs, EACCES, etc.).
pub(crate) fn ensure_uds_parent_dir(uds_path: &Path) -> Result<()> {
    let Some(parent) = uds_path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|e| {
        ClientError::connection(
            codes::UDS_DIRECTORY_CREATE_FAILED,
            messages::UDS_DIRECTORY_CREATE_FAILED,
            [
                (keys::INPUT, parent.display().to_string()),
                (keys::CAUSE, e.to_string()),
            ],
        )
    })
}

/// Bind a Unix domain socket, surfacing a structured `UDS_BIND_FAILED`
/// error rather than panicking.
pub(crate) fn bind_uds_listener(uds_path: &Path) -> Result<tokio::net::UnixListener> {
    tokio::net::UnixListener::bind(uds_path).map_err(|e| {
        ClientError::connection(
            codes::UDS_BIND_FAILED,
            messages::UDS_BIND_FAILED,
            [
                (keys::INPUT, uds_path.display().to_string()),
                (keys::CAUSE, e.to_string()),
            ],
        )
    })
}

/// A bound listener for either transport.
/// Bind a TCP listener, surfacing a structured `TCP_BIND_FAILED` error.
pub(crate) async fn bind_tcp_listener(addr: SocketAddr) -> Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        ClientError::connection(
            codes::TCP_BIND_FAILED,
            messages::TCP_BIND_FAILED,
            [
                (keys::INPUT, addr.to_string()),
                (keys::CAUSE, e.to_string()),
            ],
        )
    })
}

/// Future that resolves on the first SIGINT (Ctrl+C) or, on Unix, SIGTERM.
///
/// Wired into `Server::serve_with_shutdown` so the server drains in-flight
/// requests on a signal instead of being killed mid-stream by the runtime.
pub(crate) async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            warn!(error = %e, "failed to install ctrl_c handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                warn!(error = %e, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

// ---------------------------------------------------------------------------
// Per-kind runners
// ---------------------------------------------------------------------------

/// Serve a command-handler router until SIGINT / SIGTERM.
pub async fn run_command_handler_server(
    router: CommandHandlerRouter,
    default_port: u16,
) -> Result<()> {
    run_server(default_port, Built::CommandHandler(router)).await
}

/// Serve a saga router until SIGINT / SIGTERM. Sync targets get an
/// output-domain readiness probe; async targets rely on the bus probe
/// (`ANGZARR_BUS_ENDPOINT`).
pub async fn run_saga_server(router: SagaRouter, default_port: u16) -> Result<()> {
    run_server(default_port, Built::Saga(router)).await
}

/// Serve a projector router until SIGINT / SIGTERM.
pub async fn run_projector_server(
    router: crate::router::ProjectorRouter,
    default_port: u16,
) -> Result<()> {
    run_server(default_port, Built::Projector(router)).await
}

/// Serve a process-manager router until SIGINT / SIGTERM. Sync targets get
/// an output-domain readiness probe; async targets rely on the bus probe.
pub async fn run_process_manager_server(
    router: ProcessManagerRouter,
    default_port: u16,
) -> Result<()> {
    run_server(default_port, Built::ProcessManager(router)).await
}

/// Serve an upcaster router until SIGINT / SIGTERM.
pub async fn run_upcaster_server(
    router: crate::router::routers::UpcasterRouter,
    default_port: u16,
) -> Result<()> {
    run_server(default_port, Built::Upcaster(router)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_bind_env() {
        env::remove_var(ENV_BIND_ADDRESS);
    }

    const TRANSPORT_VARS: &[&str] = &[
        "TRANSPORT_TYPE",
        "UDS_BASE_PATH",
        "SERVICE_NAME",
        "DOMAIN",
        "SAGA_NAME",
        "PROJECTOR_NAME",
        "PORT",
        "GRPC_PORT",
    ];

    fn with_transport_env(vars: &[(&str, &str)], f: impl FnOnce()) {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for v in TRANSPORT_VARS {
            env::remove_var(v);
        }
        for (k, v) in vars {
            env::set_var(k, v);
        }
        f();
        for v in TRANSPORT_VARS {
            env::remove_var(v);
        }
    }

    #[test]
    fn transport_type_uds_uses_python_runner_defaults() {
        with_transport_env(&[("TRANSPORT_TYPE", "uds")], || {
            let cfg = ServerConfig::from_env(50052);
            assert_eq!(
                cfg.uds_path,
                Some(PathBuf::from("/tmp/angzarr/business.sock"))
            );
        });
    }

    #[test]
    fn transport_type_uds_qualifies_by_domain_then_saga_then_projector() {
        with_transport_env(
            &[
                ("TRANSPORT_TYPE", "UDS"),
                ("UDS_BASE_PATH", "/run/az"),
                ("SERVICE_NAME", "saga"),
                ("SAGA_NAME", "fulfillment"),
                ("PROJECTOR_NAME", "ignored"),
            ],
            || {
                let cfg = ServerConfig::from_env(50052);
                assert_eq!(
                    cfg.uds_path,
                    Some(PathBuf::from("/run/az/saga-fulfillment.sock"))
                );
            },
        );
        with_transport_env(
            &[
                ("TRANSPORT_TYPE", "uds"),
                ("DOMAIN", "orders"),
                ("SAGA_NAME", "ignored"),
            ],
            || {
                let cfg = ServerConfig::from_env(50052);
                assert_eq!(
                    cfg.uds_path,
                    Some(PathBuf::from("/tmp/angzarr/business-orders.sock"))
                );
            },
        );
        with_transport_env(
            &[("TRANSPORT_TYPE", "uds"), ("PROJECTOR_NAME", "ledger")],
            || {
                let cfg = ServerConfig::from_env(50052);
                assert_eq!(
                    cfg.uds_path,
                    Some(PathBuf::from("/tmp/angzarr/business-ledger.sock"))
                );
            },
        );
    }

    #[test]
    fn transport_type_tcp_wins_over_uds_variables() {
        with_transport_env(
            &[
                ("TRANSPORT_TYPE", "tcp"),
                ("UDS_BASE_PATH", "/run/az"),
                ("SERVICE_NAME", "business"),
                ("DOMAIN", "orders"),
                ("PORT", "6000"),
            ],
            || {
                let cfg = ServerConfig::from_env(50052);
                assert_eq!(cfg.uds_path, None);
                assert_eq!(cfg.port, 6000);
            },
        );
    }

    #[test]
    fn without_transport_type_all_three_uds_variables_select_uds() {
        with_transport_env(
            &[
                ("UDS_BASE_PATH", "/run/az"),
                ("SERVICE_NAME", "business"),
                ("DOMAIN", "orders"),
            ],
            || {
                let cfg = ServerConfig::from_env(50052);
                assert_eq!(
                    cfg.uds_path,
                    Some(PathBuf::from("/run/az/business-orders.sock"))
                );
            },
        );
        with_transport_env(
            &[("UDS_BASE_PATH", "/run/az"), ("DOMAIN", "orders")],
            || {
                let cfg = ServerConfig::from_env(50052);
                assert_eq!(cfg.uds_path, None);
                assert_eq!(cfg.port, 50052);
            },
        );
    }

    // Audit #77: ANGZARR_BIND_ADDRESS overrides the default
    // dual-stack `[::]:{port}` composition.

    #[test]
    fn resolve_bind_address_default_is_dual_stack() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_bind_env();
        let addr = resolve_bind_address(50052);
        assert_eq!(addr, "[::]:50052");
        // Sanity: parses as a real SocketAddr.
        let _: SocketAddr = addr.parse().expect("default must parse");
    }

    #[test]
    fn resolve_bind_address_env_override_ipv4() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_bind_env();
        env::set_var(ENV_BIND_ADDRESS, "127.0.0.1:9090");
        let addr = resolve_bind_address(50052);
        assert_eq!(addr, "127.0.0.1:9090");
        let _: SocketAddr = addr.parse().expect("override must parse");
        clear_bind_env();
    }

    #[test]
    fn resolve_bind_address_env_override_ipv6_loopback() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_bind_env();
        env::set_var(ENV_BIND_ADDRESS, "[::1]:8080");
        let addr = resolve_bind_address(50052);
        assert_eq!(addr, "[::1]:8080");
        let _: SocketAddr = addr.parse().expect("ipv6 override must parse");
        clear_bind_env();
    }

    #[test]
    fn resolve_bind_address_env_override_ignores_default_port() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_bind_env();
        env::set_var(ENV_BIND_ADDRESS, "0.0.0.0:1234");
        let addr = resolve_bind_address(50052);
        // The default_port arg is irrelevant when the override is set.
        assert_eq!(addr, "0.0.0.0:1234");
        clear_bind_env();
    }

    #[test]
    fn configure_logging_installs_the_global_subscriber() {
        configure_logging();
        assert!(tracing::dispatcher::has_been_set());
        // A second call is a no-op rather than a panic.
        configure_logging();
    }

    #[test]
    fn parse_bind_address_accepts_ipv4_and_ipv6() {
        assert!(parse_bind_address("127.0.0.1:8080").is_ok());
        assert!(parse_bind_address("[::1]:9090").is_ok());
        assert!(parse_bind_address("[::]:50052").is_ok());
    }

    #[test]
    fn parse_bind_address_rejects_garbage_with_invalid_bind_address_code() {
        let err = parse_bind_address("not-a-real-address").unwrap_err();
        assert_eq!(err.code(), codes::INVALID_BIND_ADDRESS);
        // Operator should see the original input echoed back.
        if let ClientError::InvalidArgument(d) = err {
            assert_eq!(d.details[keys::INPUT], "not-a-real-address");
        } else {
            panic!("expected InvalidArgument");
        }
    }

    #[test]
    fn parse_bind_address_rejects_empty_string() {
        let err = parse_bind_address("").unwrap_err();
        assert_eq!(err.code(), codes::INVALID_BIND_ADDRESS);
    }

    #[test]
    fn ensure_uds_parent_dir_creates_missing_parent() {
        let tmpdir = std::env::temp_dir().join(format!("angzarr-uds-{}", std::process::id(),));
        // Make sure we start clean.
        let _ = std::fs::remove_dir_all(&tmpdir);
        let socket_path = tmpdir.join("nested/dir/foo.sock");
        ensure_uds_parent_dir(&socket_path).expect("must create parent");
        assert!(tmpdir.join("nested/dir").is_dir());
        let _ = std::fs::remove_dir_all(&tmpdir);
    }

    #[test]
    fn ensure_uds_parent_dir_surfaces_failure_with_structured_code() {
        // /proc is read-only on Linux; creating a directory under it
        // surfaces as UDS_DIRECTORY_CREATE_FAILED rather than panicking
        // or being silently swallowed (the previous `let _ = ...` did
        // the latter).
        let bogus = PathBuf::from("/proc/this-cannot-be-created/foo.sock");
        let result = ensure_uds_parent_dir(&bogus);
        match result {
            Err(e) => assert_eq!(e.code(), codes::UDS_DIRECTORY_CREATE_FAILED),
            Ok(()) => {
                // /proc allows directory creation in some unprivileged
                // sandboxes — skip rather than fail spuriously.
                let _ = std::fs::remove_dir_all("/proc/this-cannot-be-created");
            }
        }
    }
}
