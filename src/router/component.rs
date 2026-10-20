//! Components: a registered handler type becomes an angzarr-router
//! dispatch table, and the helpers the kind macros' generated thunks use.
//!
//! Dispatch semantics (rebuild, conflicts, upcasting) live in the
//! `angzarr-router` crate. This module only adapts typed Rust handlers to its tables and
//! translates its coded errors into [`ClientError`].

use std::cell::RefCell;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, OnceLock};

use angzarr_router::error::{CodedError, HandlerError};
use angzarr_router::rebuild::Rebuilder;
use prost::{Message, Name};
use prost_types::Any;

use crate::error::{build_status_details, ClientError, CommandRejectedError, ErrorDetail};
use crate::error_codes::{codes, keys, messages};

/// Produces a fresh handler instance; called once per dispatch.
pub type Factory<H> = Arc<dyn Fn() -> H + Send + Sync>;

/// One registered handler as an angzarr-router dispatch table.
pub enum Component {
    CommandHandler(Box<dyn angzarr_router::router::CommandHandler>),
    Saga(angzarr_router::saga::SagaDispatch),
    ProcessManager(Box<dyn angzarr_router::router::ProcessManagerHandler>),
    Projector(Box<dyn angzarr_router::router::ProjectorHandler>),
}

/// Builds one component's dispatch table for a rules version.
pub(crate) type Table = Arc<dyn Fn(&str) -> Component + Send + Sync>;

/// Builds a component's upcaster chain.
pub(crate) type UpcasterTable =
    Arc<dyn Fn() -> angzarr_router::upcaster::UpcasterDispatch + Send + Sync>;

/// A runtime router's component: how to build its table (and its upcaster
/// chain, when it has one), and the angzarr-router engine holding them for
/// one rules version. The engine is shared with the upcaster service.
pub(crate) struct Tables {
    table: Table,
    upcaster: Option<UpcasterTable>,
    pub(crate) inner: Arc<angzarr_router::router::Router>,
}

impl Tables {
    /// The engine over `table` and `upcaster`, reporting `rules_version`.
    pub(crate) fn assemble(
        table: Table,
        upcaster: Option<UpcasterTable>,
        rules_version: &str,
    ) -> Result<Self, crate::router::BuildError> {
        let inner = Arc::new(engine(&table, upcaster.as_ref(), rules_version)?);
        Ok(Tables {
            table,
            upcaster,
            inner,
        })
    }

    /// Rebuild the engine so the table reports `rules_version`. The
    /// factory is not invoked.
    pub(crate) fn stamp(&mut self, rules_version: &str) -> Result<(), crate::router::BuildError> {
        self.inner = Arc::new(engine(&self.table, self.upcaster.as_ref(), rules_version)?);
        Ok(())
    }

    /// True when the component has an upcaster chain.
    pub(crate) fn has_upcaster(&self) -> bool {
        self.upcaster.is_some()
    }

    /// The component's upcaster chain, sharing this engine, when it has
    /// one.
    pub(crate) fn upcaster_chain(&self) -> Option<UpcasterChain> {
        self.has_upcaster()
            .then(|| UpcasterChain(Arc::clone(&self.inner)))
    }
}

/// A component's upcaster chain, for the upcaster service.
pub(crate) struct UpcasterChain(Arc<angzarr_router::router::Router>);

impl UpcasterChain {
    /// Run the pages through the chain (pages of other domains pass
    /// through unchanged).
    pub(crate) fn upcast(
        &self,
        request: &crate::proto::UpcastRequest,
    ) -> Result<crate::proto::UpcastResponse, ClientError> {
        begin_dispatch();
        self.0.upcast(request).map_err(from_coded)
    }
}

fn engine(
    table: &Table,
    upcaster: Option<&UpcasterTable>,
    rules_version: &str,
) -> Result<angzarr_router::router::Router, crate::router::BuildError> {
    let builder = angzarr_router::router::RouterBuilder::new();
    let builder = match table(rules_version) {
        Component::CommandHandler(t) => builder.aggregate(t),
        Component::Saga(s) => builder.saga(s),
        Component::ProcessManager(pm) => builder.process_manager(pm),
        Component::Projector(p) => builder.projector(p),
    };
    let builder = match upcaster {
        Some(upcaster) => builder.upcaster(upcaster()),
        None => builder,
    };
    builder.build().map_err(build_error)
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

/// The router's record of a `#[handles_fact]` method's return.
pub fn fact_record<R: super::responses::IntoFactRecord>(
    recorded: R,
) -> angzarr_router::aggregate::FactRecord {
    recorded.into_fact_record()
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
/// rejection when it raised one, otherwise the gRPC status the router's
/// error table assigns (its code, message, and a canonical ErrorInfo
/// carrying the router's code and extras).
pub(crate) fn from_coded(err: CodedError) -> ClientError {
    let rejection = REJECTION.with(|r| r.borrow_mut().take());
    if let Some(rej) = rejection.filter(|r| r.code == err.code) {
        return ClientError::Rejected(rej);
    }
    let code = tonic::Code::from(err.grpc as i32);
    let details = build_status_details(code, &err.message, &err.code, Some(&err.extras), None);
    ClientError::Grpc(Box::new(tonic::Status::with_details(
        code,
        err.message,
        bytes::Bytes::from(details),
    )))
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
// The state message: its type, snapshot image and snapshot loader.
// ---------------------------------------------------------------------------

/// The state declaration for a state type, chosen at macro expansion:
/// `(&StateMessage::<S>::new()).declare(rebuilder)` declares, when `S` is
/// a protobuf message, its type (the description's `StateDescriptor`), its
/// snapshot image (the post-command state every response carries) and its
/// snapshot loader; otherwise it leaves the rebuilder as it is.
pub struct StateMessage<S>(PhantomData<S>);

impl<S> StateMessage<S> {
    pub fn new() -> Self {
        StateMessage(PhantomData)
    }
}

impl<S> Default for StateMessage<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// Selected when `S` is a protobuf message.
pub trait DeclaresMessageState<S> {
    fn declare(&self, rebuilder: Rebuilder<S>) -> Rebuilder<S>;
}

impl<S: Message + Default + Name + 'static> DeclaresMessageState<S> for StateMessage<S> {
    fn declare(&self, rebuilder: Rebuilder<S>) -> Rebuilder<S> {
        rebuilder
            .with_state_type(&S::full_name())
            .with_state_image(|state: &S| pack(state))
            .with_snapshot(snapshot_into::<S>)
    }
}

/// Selected (by autoref) when `S` is not a protobuf message: no state type,
/// no snapshot image, no snapshot.
pub trait OpaqueState<S> {
    fn declare(&self, rebuilder: Rebuilder<S>) -> Rebuilder<S>;
}

impl<S> OpaqueState<S> for &StateMessage<S> {
    fn declare(&self, rebuilder: Rebuilder<S>) -> Rebuilder<S> {
        rebuilder
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

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

    /// The coded error's code, message and extras, read back from a status.
    fn coded_parts(err: &ClientError) -> (tonic::Code, String, String, BTreeMap<String, String>) {
        let status = err.status().expect("a framework error is a status");
        let (code, extras, _cover) =
            crate::error::unpack_status_details(status.details()).expect("canonical details");
        (status.code(), status.message().to_string(), code, extras)
    }

    #[test]
    fn a_framework_error_is_the_status_the_router_assigns() {
        begin_dispatch();
        let err = from_coded(CodedError::unknown_type(
            codes::NO_HANDLER_REGISTERED,
            "/orders.Unknown",
            "/orders.Unknown",
            [("domain".to_string(), "order".to_string())],
        ));
        let (grpc, message, code, extras) = coded_parts(&err);
        assert_eq!(grpc, tonic::Code::InvalidArgument);
        assert_eq!(message, "no handler for type URL /orders.Unknown");
        assert_eq!(code, codes::NO_HANDLER_REGISTERED);
        assert_eq!(extras.get("domain").map(String::as_str), Some("order"));
        assert!(err.is_invalid_argument());
    }

    #[test]
    fn router_grpc_codes_survive() {
        let cases = [
            (
                CodedError::invalid_argument(codes::NO_HANDLER_REGISTERED, "none", []),
                tonic::Code::Unimplemented,
            ),
            (CodedError::persisted_corrupt("/a.B"), tonic::Code::DataLoss),
            (CodedError::unhandled("bug"), tonic::Code::Internal),
            (
                CodedError::stale("CONFLICT_OVERLAP", "overlap", []),
                tonic::Code::Aborted,
            ),
            (
                CodedError::invalid_argument(codes::ANY_DECODE_FAILED, "bad", []),
                tonic::Code::InvalidArgument,
            ),
        ];
        for (coded, want) in cases {
            begin_dispatch();
            let code = coded.code.clone();
            let (grpc, _message, got, _extras) = coded_parts(&from_coded(coded));
            assert_eq!((grpc, got), (want, code));
        }
    }

    #[test]
    fn a_stale_or_unrelated_rejection_is_not_returned() {
        begin_dispatch();
        let _ = rejected(rejection("ORDER_GONE"));
        begin_dispatch();
        let err = from_coded(CodedError::invalid_argument("ORDER_GONE", "framework", []));
        assert!(matches!(err, ClientError::Grpc(_)), "got {err:?}");

        let _ = rejected(rejection("ORDER_GONE"));
        let err = from_coded(CodedError::invalid_argument("OTHER", "framework", []));
        assert!(matches!(err, ClientError::Grpc(_)), "got {err:?}");
    }

    #[test]
    fn a_message_state_declares_its_type_image_and_snapshot() {
        use super::{DeclaresMessageState as _, OpaqueState as _};
        use crate::proto::Cover;
        let rebuilder = StateMessage::<Cover>::new().declare(Rebuilder::new(Cover::default));
        assert_eq!(rebuilder.state_type_url(), "/io.angzarr.v1.Cover");
        let state = Cover {
            domain: "d".into(),
            ..Default::default()
        };
        assert_eq!(rebuilder.state_image(&state), Some(pack(&state)));
        let book = crate::proto::EventBook {
            snapshot: Some(crate::proto::Snapshot {
                sequence: 2,
                state: Some(pack(&state)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (rebuilt, _) = rebuilder.rebuild(Some(&book)).expect("rebuilds");
        assert_eq!(rebuilt, state);

        #[derive(Default)]
        struct Plain;
        let opaque = (&StateMessage::<Plain>::new()).declare(Rebuilder::new(Plain::default));
        assert_eq!(opaque.state_type_url(), "");
        assert!(opaque.state_image(&Plain).is_none());
    }

    #[test]
    fn interned_strings_are_shared() {
        let a = intern("INTERN_TEST_CODE", "fallback");
        let b = intern(&String::from("INTERN_TEST_CODE"), "fallback");
        assert_eq!(a, "INTERN_TEST_CODE");
        assert!(std::ptr::eq(a, b));
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
