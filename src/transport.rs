//! Transport-mode resolver for command handler coordinator endpoints.
//!
//! Mirrors the Go (`client.go`) and Python (`client.py`) implementations so
//! the same env vars resolve to the same endpoints across languages.
//!
//! # Environment variables
//!
//! - `ANGZARR_MODE`: `"standalone"` (UDS) or `"distributed"` (K8s DNS). Default: distributed.
//! - `ANGZARR_UDS_BASE`: base path for Unix domain sockets. Default: `/tmp/angzarr`.
//! - `ANGZARR_NAMESPACE`: Kubernetes namespace. Default: `angzarr`.
//! - `ANGZARR_CH_PORT`: gRPC port for distributed mode. Default: 1310.

use std::env;

/// Transport mode for gRPC connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    /// Unix Domain Sockets for local-process communication.
    Standalone,
    /// TCP via Kubernetes DNS for cluster communication.
    Distributed,
}

/// Default UDS base directory.
pub const DEFAULT_UDS_BASE: &str = "/tmp/angzarr";
/// Default Kubernetes namespace.
pub const DEFAULT_NAMESPACE: &str = "angzarr";
/// Default command handler coordinator port.
pub const DEFAULT_CH_PORT: u16 = 1310;
/// Default transport mode when neither arg nor env var is set.
pub const DEFAULT_TRANSPORT_MODE: TransportMode = TransportMode::Distributed;

/// Env var names.
pub const ENV_MODE: &str = "ANGZARR_MODE";
pub const ENV_UDS_BASE: &str = "ANGZARR_UDS_BASE";
pub const ENV_NAMESPACE: &str = "ANGZARR_NAMESPACE";
pub const ENV_CH_PORT: &str = "ANGZARR_CH_PORT";

impl TransportMode {
    /// Resolve mode from the `ANGZARR_MODE` env var, falling back to
    /// `DEFAULT_TRANSPORT_MODE` when unset or unrecognized.
    pub fn from_env() -> Self {
        match env::var(ENV_MODE).ok().as_deref() {
            Some("standalone") => TransportMode::Standalone,
            Some("distributed") => TransportMode::Distributed,
            _ => DEFAULT_TRANSPORT_MODE,
        }
    }
}

/// Resolve a domain name to a command handler coordinator endpoint.
///
/// - `Standalone` → `{ANGZARR_UDS_BASE}/ch-{domain}.sock` (default base `/tmp/angzarr`)
/// - `Distributed` → `ch-{domain}.{ANGZARR_NAMESPACE}.svc:{ANGZARR_CH_PORT}`
///
/// When `mode` is `None`, auto-detects from `ANGZARR_MODE`.
pub fn resolve_ch_endpoint(domain: &str, mode: Option<TransportMode>) -> String {
    let mode = mode.unwrap_or_else(TransportMode::from_env);
    match mode {
        TransportMode::Standalone => {
            let base = env::var(ENV_UDS_BASE).unwrap_or_else(|_| DEFAULT_UDS_BASE.to_string());
            format!("{}/ch-{}.sock", base, domain)
        }
        TransportMode::Distributed => {
            let ns = env::var(ENV_NAMESPACE).unwrap_or_else(|_| DEFAULT_NAMESPACE.to_string());
            let port = env::var(ENV_CH_PORT)
                .ok()
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(DEFAULT_CH_PORT);
            format!("ch-{}.{}.svc:{}", domain, ns, port)
        }
    }
}
