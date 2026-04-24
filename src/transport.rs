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
/// - `Standalone` → `{uds_base}/ch-{domain}.sock` (default `/tmp/angzarr`)
/// - `Distributed` → `ch-{domain}.{namespace}.svc:{port}` (default `angzarr`, `1310`)
///
/// Resolution precedence for each value matches Python's
/// `resolve_ch_endpoint(domain, mode, *, uds_base, namespace, port)`:
/// **env var > explicit arg > hardcoded default**.
///
/// When `mode` is `None`, auto-detects from `ANGZARR_MODE`.
/// When `uds_base`/`namespace`/`port` are `None`, the env var is consulted
/// and then the hardcoded default is used.
pub fn resolve_ch_endpoint(
    domain: &str,
    mode: Option<TransportMode>,
    uds_base: Option<&str>,
    namespace: Option<&str>,
    port: Option<u16>,
) -> String {
    let mode = mode.unwrap_or_else(TransportMode::from_env);
    match mode {
        TransportMode::Standalone => {
            let base = env::var(ENV_UDS_BASE)
                .ok()
                .or_else(|| uds_base.map(|s| s.to_string()))
                .unwrap_or_else(|| DEFAULT_UDS_BASE.to_string());
            format!("{}/ch-{}.sock", base, domain)
        }
        TransportMode::Distributed => {
            let ns = env::var(ENV_NAMESPACE)
                .ok()
                .or_else(|| namespace.map(|s| s.to_string()))
                .unwrap_or_else(|| DEFAULT_NAMESPACE.to_string());
            let p = env::var(ENV_CH_PORT)
                .ok()
                .and_then(|p| p.parse::<u16>().ok())
                .or(port)
                .unwrap_or(DEFAULT_CH_PORT);
            format!("ch-{}.{}.svc:{}", domain, ns, p)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests manipulate process-wide env vars; a mutex serializes them so the
    // test binary can still run with the default thread parallelism.
    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        for v in [ENV_MODE, ENV_UDS_BASE, ENV_NAMESPACE, ENV_CH_PORT] {
            env::remove_var(v);
        }
    }

    #[test]
    fn standalone_uds_base_defaults_when_no_env_no_arg() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let ep = resolve_ch_endpoint("player", Some(TransportMode::Standalone), None, None, None);
        assert_eq!(ep, "/tmp/angzarr/ch-player.sock");
    }

    #[test]
    fn standalone_arg_beats_default_when_no_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let ep = resolve_ch_endpoint(
            "player",
            Some(TransportMode::Standalone),
            Some("/srv/sockets"),
            None,
            None,
        );
        assert_eq!(ep, "/srv/sockets/ch-player.sock");
    }

    #[test]
    fn standalone_env_beats_arg() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        env::set_var(ENV_UDS_BASE, "/env/base");
        let ep = resolve_ch_endpoint(
            "player",
            Some(TransportMode::Standalone),
            Some("/arg/base"),
            None,
            None,
        );
        assert_eq!(ep, "/env/base/ch-player.sock");
        clear_env();
    }

    #[test]
    fn distributed_ns_and_port_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let ep = resolve_ch_endpoint("player", Some(TransportMode::Distributed), None, None, None);
        assert_eq!(ep, "ch-player.angzarr.svc:1310");
    }

    #[test]
    fn distributed_args_beat_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let ep = resolve_ch_endpoint(
            "player",
            Some(TransportMode::Distributed),
            None,
            Some("my-ns"),
            Some(2222),
        );
        assert_eq!(ep, "ch-player.my-ns.svc:2222");
    }

    #[test]
    fn distributed_env_beats_args() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        env::set_var(ENV_NAMESPACE, "env-ns");
        env::set_var(ENV_CH_PORT, "5555");
        let ep = resolve_ch_endpoint(
            "player",
            Some(TransportMode::Distributed),
            None,
            Some("arg-ns"),
            Some(2222),
        );
        assert_eq!(ep, "ch-player.env-ns.svc:5555");
        clear_env();
    }
}
