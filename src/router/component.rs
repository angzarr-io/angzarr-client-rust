//! Components: each registered handler type becomes an angzarr-router
//! dispatch table, and the helpers the kind macros' generated thunks use.
//!
//! Dispatch semantics (rebuild, fill-only stamping, deferred commands,
//! compensation routing, fan-out and merging) live in the `angzarr-router`
//! crate. This module only adapts typed Rust handlers to its tables and
//! translates its coded errors into [`ClientError`].

use std::cell::RefCell;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, OnceLock};

use angzarr_router::error::{CodedError, GrpcCode, HandlerError};
use angzarr_router::rebuild::Rebuilder;
use prost::{Message, Name};
use prost_types::Any;

use crate::error::{ClientError, CommandRejectedError, ErrorDetail};
use crate::error_codes::{codes, keys, messages};

/// Produces a fresh handler instance; called once per dispatch.
pub type Factory<H> = Arc<dyn Fn() -> H + Send + Sync>;

/// One registered handler as an angzarr-router dispatch table.
pub enum Component {
    CommandHandler(Box<dyn angzarr_router::router::CommandHandler>),
    Saga(angzarr_router::saga::SagaDispatch),
    ProcessManager(Box<dyn angzarr_router::router::ProcessManagerHandler>),
    Projector(Box<dyn angzarr_router::router::ProjectorHandler>),
    Upcaster(angzarr_router::upcaster::UpcasterDispatch),
}

// ---------------------------------------------------------------------------
// Errors crossing the router boundary.
// ---------------------------------------------------------------------------

thread_local! {
    /// The business rejection a handler raised during the current dispatch.
    /// Dispatch is synchronous on one thread, so the router's coded error
    /// can be turned back into the handler's own `CommandRejectedError`.
    static REJECTION: RefCell<Option<CommandRejectedError>> = const { RefCell::new(None) };
}

/// Forget any rejection recorded by an earlier dispatch on this thread.
pub(crate) fn begin_dispatch() {
    REJECTION.with(|r| r.borrow_mut().take());
}

/// A handler's business rejection, as the router's handler error.
///
/// The rejection itself is kept for `from_coded`, which returns it
/// (status code and all) once the router has unwound; the coded error only
/// carries its code through the router.
pub fn rejected(rej: CommandRejectedError) -> HandlerError {
    let extras: Vec<(String, String)> = rej.details.clone().into_iter().collect();
    let coded = CodedError::rejection_precondition_failed(rej.code, rej.message, extras);
    REJECTION.with(|r| *r.borrow_mut() = Some(rej));
    HandlerError::Coded(coded)
}

/// Decode a payload `Any` as `T`; a malformed payload is ANY_DECODE_FAILED.
pub fn decode<T: Message + Default>(any: &Any) -> Result<T, HandlerError> {
    T::decode(any.value.as_slice()).map_err(|e| {
        HandlerError::Coded(CodedError::invalid_argument(
            codes::ANY_DECODE_FAILED,
            messages::ANY_DECODE_FAILED,
            [
                (keys::TYPE_URL.to_string(), any.type_url.clone()),
                (keys::CAUSE.to_string(), e.to_string()),
            ],
        ))
    })
}

/// Decode an event `Any` as `T` for an applier; failures surface from the
/// rebuild as PERSISTED_EVENT_CORRUPT.
pub fn decode_applied<T: Message + Default>(
    any: &Any,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
    T::decode(any.value.as_slice()).map_err(|e| Box::new(e) as _)
}

/// Pack `msg` into an `Any` under its `/`-prefixed type URL.
pub fn pack<T: Message + Name>(msg: &T) -> Any {
    Any {
        type_url: crate::full_type_url::<T>(),
        value: msg.encode_to_vec(),
    }
}

/// Intern a dynamic string so it can ride in the `&'static str` fields of
/// [`ErrorDetail`]. Router codes and messages come from a small fixed set;
/// past a bound, the generic text is used instead of growing the set.
fn intern(s: &str, fallback: &'static str) -> &'static str {
    static SET: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut set = SET
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(found) = set.get(s) {
        return found;
    }
    if set.len() >= 1024 {
        return fallback;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.insert(leaked);
    leaked
}

/// The router's coded error as a [`ClientError`]: the handler's own
/// rejection when it raised one, otherwise an `InvalidArgument` detail
/// carrying the router's code, message and extras.
pub(crate) fn from_coded(err: CodedError) -> ClientError {
    let rejection = REJECTION.with(|r| r.borrow_mut().take());
    if let Some(rej) = rejection.filter(|r| r.code == err.code) {
        return ClientError::Rejected(rej);
    }
    ClientError::InvalidArgument(ErrorDetail {
        code: intern(&err.code, codes::UNHANDLED_HANDLER_ERROR),
        message: intern(&err.message, messages::UNHANDLED_HANDLER_ERROR),
        details: err.extras,
    })
}

/// The gRPC status code the router's error table assigns to a code; codes
/// it does not classify are `INVALID_ARGUMENT`.
pub(crate) fn grpc_code_for(code: &str) -> tonic::Code {
    let probe = CodedError::invalid_argument(code, "", []);
    match (code, probe.grpc) {
        (codes::UNHANDLED_HANDLER_ERROR, _) => tonic::Code::Internal,
        (_, GrpcCode::Unimplemented) => tonic::Code::Unimplemented,
        (_, GrpcCode::DataLoss) => tonic::Code::DataLoss,
        _ => tonic::Code::InvalidArgument,
    }
}

/// A router build error as a [`crate::router::BuildError`].
pub(crate) fn build_error(err: CodedError) -> crate::router::BuildError {
    crate::router::BuildError::InvalidComponent(ErrorDetail {
        code: intern(&err.code, codes::UNHANDLED_HANDLER_ERROR),
        message: intern(&err.message, messages::UNHANDLED_HANDLER_ERROR),
        details: err.extras,
    })
}

// ---------------------------------------------------------------------------
// Snapshots and replay.
// ---------------------------------------------------------------------------

/// Snapshot loading for a state type, chosen at macro expansion:
/// `(&SnapshotState::<S>::new()).snapshot_loader()` yields a loader when
/// `S` is a protobuf message (the snapshot's state decodes into it) and
/// `None` otherwise (the aggregate never snapshots).
pub struct SnapshotState<S>(PhantomData<S>);

impl<S> SnapshotState<S> {
    pub fn new() -> Self {
        SnapshotState(PhantomData)
    }
}

impl<S> Default for SnapshotState<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// A snapshot loader: replaces the fresh state with the snapshot's state.
pub type SnapshotLoader<S> =
    Box<dyn Fn(&mut S, &Any) -> Result<(), Box<dyn std::error::Error + Send + Sync>> + Send + Sync>;

/// Selected when `S` is a protobuf message.
pub trait LoadsSnapshot<S> {
    fn snapshot_loader(&self) -> Option<SnapshotLoader<S>>;
}

impl<S: Message + Default + Name + 'static> LoadsSnapshot<S> for SnapshotState<S> {
    fn snapshot_loader(&self) -> Option<SnapshotLoader<S>> {
        Some(Box::new(snapshot_into::<S>))
    }
}

/// Selected (by autoref) when `S` is not a protobuf message.
pub trait IgnoresSnapshot<S> {
    fn snapshot_loader(&self) -> Option<SnapshotLoader<S>>;
}

impl<S> IgnoresSnapshot<S> for &SnapshotState<S> {
    fn snapshot_loader(&self) -> Option<SnapshotLoader<S>> {
        None
    }
}

/// Decode a snapshot state of type `S` into `state`. An empty value keeps
/// the fresh state; a snapshot of another type is an error.
fn snapshot_into<S: Message + Default + Name>(
    state: &mut S,
    any: &Any,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if any.value.is_empty() {
        return Ok(());
    }
    if !crate::type_url_is::<S>(&any.type_url) {
        return Err(format!("snapshot state {} is not {}", any.type_url, S::full_name()).into());
    }
    *state = S::decode(any.value.as_slice())?;
    Ok(())
}

/// Attach the snapshot loader (if `S` has one) to a rebuilder.
pub fn with_snapshot<S: 'static>(
    rebuilder: Rebuilder<S>,
    loader: Option<SnapshotLoader<S>>,
) -> Rebuilder<S> {
    match loader {
        Some(loader) => rebuilder.with_snapshot(loader),
        None => rebuilder,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejection(code: &'static str) -> CommandRejectedError {
        CommandRejectedError::not_found(code, "gone", [("k", "v")])
    }

    #[test]
    fn a_handler_rejection_survives_the_router_unchanged() {
        begin_dispatch();
        let HandlerError::Coded(coded) = rejected(rejection("ORDER_GONE")) else {
            panic!("rejections are coded");
        };
        assert_eq!(coded.code, "ORDER_GONE");
        match from_coded(coded) {
            ClientError::Rejected(rej) => {
                assert_eq!(rej.code, "ORDER_GONE");
                assert_eq!(rej.status_code, "NOT_FOUND");
                assert_eq!(rej.details.get("k").map(String::as_str), Some("v"));
            }
            other => panic!("expected the rejection back, got {other:?}"),
        }
    }

    #[test]
    fn a_framework_error_is_an_invalid_argument_detail() {
        begin_dispatch();
        let err = from_coded(CodedError::invalid_argument(
            codes::NO_HANDLER_REGISTERED,
            "Unknown command type",
            [("domain".to_string(), "order".to_string())],
        ));
        let ClientError::InvalidArgument(detail) = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert_eq!(detail.code, codes::NO_HANDLER_REGISTERED);
        assert_eq!(detail.message, "Unknown command type");
        assert_eq!(
            detail.details.get("domain").map(String::as_str),
            Some("order")
        );
    }

    #[test]
    fn a_stale_or_unrelated_rejection_is_not_returned() {
        begin_dispatch();
        let _ = rejected(rejection("ORDER_GONE"));
        begin_dispatch();
        let err = from_coded(CodedError::invalid_argument("ORDER_GONE", "framework", []));
        assert!(
            matches!(err, ClientError::InvalidArgument(_)),
            "got {err:?}"
        );

        let _ = rejected(rejection("ORDER_GONE"));
        let err = from_coded(CodedError::invalid_argument("OTHER", "framework", []));
        assert!(
            matches!(err, ClientError::InvalidArgument(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn interned_strings_are_shared() {
        let a = intern("INTERN_TEST_CODE", "fallback");
        let b = intern(&String::from("INTERN_TEST_CODE"), "fallback");
        assert_eq!(a, "INTERN_TEST_CODE");
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn grpc_codes_follow_the_router_table() {
        assert_eq!(
            grpc_code_for(codes::NO_HANDLER_REGISTERED),
            tonic::Code::Unimplemented
        );
        assert_eq!(grpc_code_for("NO_UNDO_HANDLER"), tonic::Code::Unimplemented);
        assert_eq!(
            grpc_code_for("PERSISTED_EVENT_CORRUPT"),
            tonic::Code::DataLoss
        );
        assert_eq!(
            grpc_code_for(codes::UNHANDLED_HANDLER_ERROR),
            tonic::Code::Internal
        );
        assert_eq!(
            grpc_code_for(codes::ANY_DECODE_FAILED),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn decode_failures_are_any_decode_failed() {
        let bad = Any {
            type_url: "/io.angzarr.v1.Cover".into(),
            value: vec![0xff, 0xff],
        };
        let Err(HandlerError::Coded(coded)) = decode::<crate::proto::Cover>(&bad) else {
            panic!("expected a decode failure");
        };
        assert_eq!(coded.code, codes::ANY_DECODE_FAILED);
        assert_eq!(
            coded.extras.get(keys::TYPE_URL).map(String::as_str),
            Some("/io.angzarr.v1.Cover")
        );
    }
}
