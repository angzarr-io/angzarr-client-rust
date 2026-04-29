//! Typed runtime routers — output of [`Router::build`].
//!
//! Each struct carries the type-erased factories the builder collected and
//! will grow a `dispatch()` method in later rounds (R6 for command handler,
//! R11 saga, R12 process manager, R13 projector).
//!
//! These types share short names with the legacy generic routers in
//! `router/mod.rs`. During the Tier 5 transition they are reached via the
//! `router::runtime::` path; R15 cleanup deletes the legacy types and
//! re-exports these at `router::*`.
//!
//! [`Router::build`]: crate::router::Router::build

use crate::proto::{
    business_response, BusinessResponse, CommandBook, ContextualCommand, Cover, EventBook,
    Notification, ProcessManagerHandleRequest, ProcessManagerHandleResponse, Projection,
    RejectionNotification, SagaHandleRequest, SagaResponse,
};
use crate::router::builder::Factory;
use crate::router::{Handler, HandlerConfig, HandlerRequest, HandlerResponse};
use crate::ClientError;
use prost::Message;
use std::sync::OnceLock;

/// Audit #86: stamp `source.edition` onto every outgoing book's cover.
/// **Always-override semantics** — handler choices are overwritten so
/// the framework guarantees timeline consistency on cross-domain
/// emissions.
fn propagate_edition_into_books(
    source: &Cover,
    commands: &mut [CommandBook],
    events: &mut [EventBook],
) {
    for book in commands.iter_mut() {
        if let Some(cover) = book.cover.as_mut() {
            cover.propagate_edition_from(source);
        }
    }
    for book in events.iter_mut() {
        if let Some(cover) = book.cover.as_mut() {
            cover.propagate_edition_from(source);
        }
    }
}

/// Runtime router built from one-or-more aggregate factories.
#[derive(Debug)]
pub struct CommandHandlerRouter {
    pub(crate) factories: Vec<Factory>,
    /// Audit #42: cached identifier so `name()` is infallible after the
    /// first call and the factory is invoked at most once per router
    /// lifetime — matching Python's zero-factory-call property.
    pub(crate) cached_name: OnceLock<String>,
}

impl CommandHandlerRouter {
    /// Dispatch a contextual command, fanning out to every registered handler
    /// whose `#[handles]` metadata matches the command's type URL.
    ///
    /// Semantics (R8):
    /// - All matching handlers run in registration order.
    /// - Emitted event pages are concatenated.
    /// - Each handler rebuilds its own state independently (handled by the
    ///   per-instance `Handler::dispatch` body the macro emits).
    /// - Factory invocation count equals the number of matched handlers.
    ///
    /// Sequence threading across merged output lands in R9; rejection
    /// routing in R10.
    pub fn dispatch(&self, cmd: ContextualCommand) -> Result<BusinessResponse, ClientError> {
        let type_url = extract_command_type_url(&cmd)?;

        // R10: Notification → rejection flow, matched against each handler's
        // `#[rejected(domain, command)]` set instead of `#[handles]`.
        if type_url == crate::full_type_url::<Notification>() {
            return self.dispatch_rejection(cmd);
        }

        // Audit finding #46: routing key is `(domain, type_url)`, not
        // `type_url` alone. Audit finding #18: at most one handler per
        // `(domain, type_url)` — enforced at build time, so this loop
        // either finds zero or one match.
        let cover_domain = cmd
            .command
            .as_ref()
            .and_then(|cb| cb.cover.as_ref())
            .map(|c| c.domain.as_str())
            .unwrap_or("")
            .to_string();

        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            let (declared_domain, handles) = match handler.config() {
                HandlerConfig::CommandHandler {
                    domain, handled, ..
                } => (domain, handled),
                _ => continue,
            };
            if declared_domain != cover_domain {
                continue;
            }
            if !handles.iter().any(|u| u == &type_url) {
                continue;
            }

            let response = handler.dispatch(HandlerRequest::CommandHandler(cmd))?;
            let HandlerResponse::CommandHandler(br) = response else {
                return Err(ClientError::invalid_argument(
                    crate::error_codes::codes::HANDLER_WRONG_RESPONSE_KIND,
                    crate::error_codes::messages::HANDLER_WRONG_RESPONSE_KIND,
                    [(crate::error_codes::keys::EXPECTED_KIND, "CommandHandler")],
                ));
            };
            return Ok(br);
        }

        // No matching handler. Mirror Python's wording
        // (`dispatch.py:246-249`): include both domain and type_url so
        // the caller can distinguish "wrong domain" from "wrong command
        // type". P2.6 / audit finding #11.
        let domain = cmd
            .command
            .as_ref()
            .and_then(|cb| cb.cover.as_ref())
            .map(|c| c.domain.as_str())
            .unwrap_or("<missing>");
        Err(ClientError::invalid_argument(
            crate::error_codes::codes::NO_HANDLER_REGISTERED,
            crate::error_codes::messages::NO_HANDLER_REGISTERED,
            [
                (crate::error_codes::keys::DOMAIN, domain.to_string()),
                (crate::error_codes::keys::TYPE_URL, type_url.clone()),
            ],
        ))
    }

    /// Notification path: fan out to every handler whose `#[rejected]` set
    /// includes the rejection key `(target_domain, target_command_suffix)`.
    /// Compensation events concatenate across matched handlers.
    fn dispatch_rejection(&self, cmd: ContextualCommand) -> Result<BusinessResponse, ClientError> {
        let (target_domain, target_command_suffix) = extract_rejection_key(&cmd)?;

        let initial_next_seq = cmd.events.as_ref().map(|eb| eb.next_sequence).unwrap_or(0);
        let mut running_seq = initial_next_seq;
        let mut merged = EventBook {
            next_sequence: initial_next_seq,
            ..Default::default()
        };

        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            let rejected = match handler.config() {
                HandlerConfig::CommandHandler { rejected, .. } => rejected,
                _ => continue,
            };
            let matches = rejected
                .iter()
                .any(|(d, c)| d == &target_domain && c == &target_command_suffix);
            if !matches {
                continue;
            }

            let mut scoped_cmd = cmd.clone();
            if let Some(eb) = scoped_cmd.events.as_mut() {
                eb.next_sequence = running_seq;
            }

            let response = handler.dispatch(HandlerRequest::CommandHandler(scoped_cmd))?;
            let HandlerResponse::CommandHandler(br) = response else {
                return Err(ClientError::invalid_argument(
                    crate::error_codes::codes::HANDLER_WRONG_RESPONSE_KIND,
                    crate::error_codes::messages::HANDLER_WRONG_RESPONSE_KIND,
                    [(crate::error_codes::keys::EXPECTED_KIND, "CommandHandler")],
                ));
            };
            if let Some(business_response::Result::Events(events)) = br.result {
                running_seq += events.pages.len() as u32;
                merged.pages.extend(events.pages);
            }
        }

        merged.next_sequence = running_seq;
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(merged)),
        })
    }
}

/// Decode the incoming Notification + RejectionNotification and return the
/// rejection key `(target_domain, target_command_suffix)` to match against
/// `#[rejected(domain, command)]` entries.
fn extract_rejection_key(cmd: &ContextualCommand) -> Result<(String, String), ClientError> {
    use crate::error_codes::{codes, keys, messages};

    let book = cmd.command.as_ref().ok_or_else(|| {
        ClientError::invalid_argument(
            codes::MISSING_COMMAND_BOOK,
            messages::MISSING_COMMAND_BOOK,
            std::iter::empty::<(String, String)>(),
        )
    })?;
    let page = book.pages.first().ok_or_else(|| {
        ClientError::invalid_argument(
            codes::MISSING_COMMAND_PAGE,
            messages::MISSING_COMMAND_PAGE,
            std::iter::empty::<(String, String)>(),
        )
    })?;
    let payload = match &page.payload {
        Some(crate::proto::command_page::Payload::Command(c)) => c,
        _ => {
            return Err(ClientError::invalid_argument(
                codes::MISSING_COMMAND_PAYLOAD,
                messages::MISSING_COMMAND_PAYLOAD,
                std::iter::empty::<(String, String)>(),
            ));
        }
    };
    let notif = Notification::decode(payload.value.as_slice()).map_err(|e| {
        ClientError::invalid_argument(
            codes::NOTIFICATION_DECODE_FAILED,
            messages::NOTIFICATION_DECODE_FAILED,
            [(keys::CAUSE, e.to_string())],
        )
    })?;
    let rejection = match notif.payload.as_ref() {
        Some(p) => RejectionNotification::decode(p.value.as_slice()).map_err(|e| {
            ClientError::invalid_argument(
                codes::REJECTION_NOTIFICATION_DECODE_FAILED,
                messages::REJECTION_NOTIFICATION_DECODE_FAILED,
                [(keys::CAUSE, e.to_string())],
            )
        })?,
        None => RejectionNotification::default(),
    };
    let domain = rejection
        .rejected_command
        .as_ref()
        .and_then(|cb| cb.cover.as_ref().map(|c| c.domain.clone()))
        .unwrap_or_default();
    let command_suffix = rejection
        .rejected_command
        .as_ref()
        .and_then(|cb| {
            cb.pages.first().and_then(|p| match &p.payload {
                Some(crate::proto::command_page::Payload::Command(a)) => Some(a.type_url.clone()),
                _ => None,
            })
        })
        .map(|url| {
            url.rsplit('/')
                .next()
                .unwrap_or("")
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();
    Ok((domain, command_suffix))
}

/// Pull the command's `type_url` from a `ContextualCommand`, or error on
/// any missing nesting layer.
fn extract_command_type_url(cmd: &ContextualCommand) -> Result<String, ClientError> {
    use crate::error_codes::{codes, messages};

    let book = cmd.command.as_ref().ok_or_else(|| {
        ClientError::invalid_argument(
            codes::MISSING_COMMAND_BOOK,
            messages::MISSING_COMMAND_BOOK,
            std::iter::empty::<(String, String)>(),
        )
    })?;
    let page = book.pages.first().ok_or_else(|| {
        ClientError::invalid_argument(
            codes::MISSING_COMMAND_PAGE,
            messages::MISSING_COMMAND_PAGE,
            std::iter::empty::<(String, String)>(),
        )
    })?;
    let payload = match &page.payload {
        Some(crate::proto::command_page::Payload::Command(c)) => c,
        _ => {
            return Err(ClientError::invalid_argument(
                codes::MISSING_COMMAND_PAYLOAD,
                messages::MISSING_COMMAND_PAYLOAD,
                std::iter::empty::<(String, String)>(),
            ));
        }
    };
    Ok(payload.type_url.clone())
}

/// Runtime router built from one-or-more saga factories.
#[derive(Debug)]
pub struct SagaRouter {
    pub(crate) factories: Vec<Factory>,
    /// Cached saga `name` (audit #42).
    pub(crate) cached_name: OnceLock<String>,
}

impl SagaRouter {
    /// Dispatch a saga-handle request to every registered saga whose
    /// `#[handles]` set matches the triggering event's type URL.
    ///
    /// Matching handlers run in registration order; their emitted
    /// `SagaResponse.commands` and `SagaResponse.events` concatenate into
    /// the merged response.
    pub fn dispatch(&self, request: SagaHandleRequest) -> Result<SagaResponse, ClientError> {
        let type_url = extract_saga_event_type_url(&request)?;

        // Audit finding #46: filter by the saga's declared `source` domain
        // before type_url matching. Mirrors Python's
        // `dispatch_saga:387-389` (`if cls.__angzarr_meta__.get("source")
        // != source_domain: continue`). The source-book cover supplies
        // the runtime domain.
        let source_cover = request
            .source
            .as_ref()
            .and_then(|eb| eb.cover.as_ref())
            .cloned();
        let source_domain = source_cover
            .as_ref()
            .map(|c| c.domain.as_str())
            .unwrap_or("")
            .to_string();

        let mut merged = SagaResponse::default();
        let mut matched = 0u32;

        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            let (declared_source, handles) = match handler.config() {
                HandlerConfig::Saga {
                    source, handled, ..
                } => (source, handled),
                _ => continue,
            };
            if declared_source != source_domain {
                continue;
            }
            if !handles.iter().any(|u| u == &type_url) {
                continue;
            }

            let response = handler.dispatch(HandlerRequest::Saga(request.clone()))?;
            let HandlerResponse::Saga(mut sr) = response else {
                return Err(ClientError::invalid_argument(
                    crate::error_codes::codes::HANDLER_WRONG_RESPONSE_KIND,
                    crate::error_codes::messages::HANDLER_WRONG_RESPONSE_KIND,
                    [(crate::error_codes::keys::EXPECTED_KIND, "Saga")],
                ));
            };
            // Audit #86: always-override edition propagation. Every
            // outgoing CommandBook / EventBook inherits the source
            // cover's edition, even if the handler set its own.
            if let Some(src) = source_cover.as_ref() {
                propagate_edition_into_books(src, &mut sr.commands, &mut sr.events);
            }
            merged.commands.extend(sr.commands);
            merged.events.extend(sr.events);
            matched += 1;
        }

        // P2.5 / audit finding #36: "no handler matched" is a normal
        // runtime condition (no saga subscribed to this event type), not
        // a failure. Log at info-level for observability and return an
        // empty SagaResponse — matches Python's silent return.
        if matched == 0 {
            tracing::info!(
                type_url = %type_url,
                "no saga handler registered for event type — returning empty SagaResponse"
            );
        }

        Ok(merged)
    }
}

fn extract_saga_event_type_url(request: &SagaHandleRequest) -> Result<String, ClientError> {
    use crate::convert::TYPE_URL_PREFIX;
    use crate::error_codes::{codes, keys, messages};

    let book = request.source.as_ref().ok_or_else(|| {
        ClientError::invalid_argument(
            codes::MISSING_SAGA_SOURCE,
            messages::MISSING_SAGA_SOURCE,
            std::iter::empty::<(String, String)>(),
        )
    })?;
    let page = book.pages.last().ok_or_else(|| {
        ClientError::invalid_argument(
            codes::EMPTY_SAGA_SOURCE,
            messages::EMPTY_SAGA_SOURCE,
            std::iter::empty::<(String, String)>(),
        )
    })?;
    let payload = match &page.payload {
        Some(crate::proto::event_page::Payload::Event(e)) => e,
        _ => {
            return Err(ClientError::invalid_argument(
                codes::MISSING_SAGA_EVENT_PAYLOAD,
                messages::MISSING_SAGA_EVENT_PAYLOAD,
                std::iter::empty::<(String, String)>(),
            ));
        }
    };
    // P2.5 / audit finding #35: explicit-over-tolerant. Reject empty or
    // non-googleapis type URLs the same way Python does — surfaces
    // wire-protocol violations as INVALID_ARGUMENT rather than letting
    // them fall through to the generic "no handler registered" path.
    if payload.type_url.is_empty() || !payload.type_url.starts_with(TYPE_URL_PREFIX) {
        return Err(ClientError::invalid_argument(
            codes::SAGA_INVALID_TYPE_URL,
            messages::SAGA_INVALID_TYPE_URL,
            [(keys::TYPE_URL, payload.type_url.clone())],
        ));
    }
    Ok(payload.type_url.clone())
}

/// Runtime router built from one-or-more process-manager factories.
#[derive(Debug)]
pub struct ProcessManagerRouter {
    pub(crate) factories: Vec<Factory>,
    /// Cached PM `name` (audit #42).
    pub(crate) cached_name: OnceLock<String>,
}

impl ProcessManagerRouter {
    /// Dispatch a PM-handle request to every registered PM whose `#[handles]`
    /// set matches the triggering event's type URL. Handlers run in
    /// registration order; their emitted `commands` / `facts` / `process_events`
    /// concatenate into the merged response.
    pub fn dispatch(
        &self,
        request: ProcessManagerHandleRequest,
    ) -> Result<ProcessManagerHandleResponse, ClientError> {
        let type_url = extract_pm_event_type_url(&request)?;

        // Audit finding #46: filter by the PM's declared `sources` list
        // before type_url matching. Mirrors Python's
        // `dispatch_process_manager:444-446` (`if trigger_domain not in
        // sources: continue`). The trigger-book cover supplies the
        // runtime domain.
        let trigger_cover = request
            .trigger
            .as_ref()
            .and_then(|eb| eb.cover.as_ref())
            .cloned();
        let trigger_domain = trigger_cover
            .as_ref()
            .map(|c| c.domain.as_str())
            .unwrap_or("")
            .to_string();

        let mut merged = ProcessManagerHandleResponse::default();
        let mut matched = 0u32;

        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            let (declared_sources, handles) = match handler.config() {
                HandlerConfig::ProcessManager {
                    sources, handled, ..
                } => (sources, handled),
                _ => continue,
            };
            if !declared_sources.iter().any(|s| s == &trigger_domain) {
                continue;
            }
            if !handles.iter().any(|u| u == &type_url) {
                continue;
            }

            let response = handler.dispatch(HandlerRequest::ProcessManager(request.clone()))?;
            let HandlerResponse::ProcessManager(mut pr) = response else {
                return Err(ClientError::invalid_argument(
                    crate::error_codes::codes::HANDLER_WRONG_RESPONSE_KIND,
                    crate::error_codes::messages::HANDLER_WRONG_RESPONSE_KIND,
                    [(crate::error_codes::keys::EXPECTED_KIND, "ProcessManager")],
                ));
            };
            // Audit #86: always-override edition propagation. Every
            // outgoing CommandBook / EventBook inherits the trigger
            // cover's edition, even if the handler set its own.
            if let Some(trg) = trigger_cover.as_ref() {
                propagate_edition_into_books(trg, &mut pr.commands, &mut pr.facts);
                if let Some(pe) = pr.process_events.as_mut() {
                    if let Some(cover) = pe.cover.as_mut() {
                        cover.propagate_edition_from(trg);
                    }
                }
            }
            merged.commands.extend(pr.commands);
            merged.facts.extend(pr.facts);
            if let Some(evts) = pr.process_events {
                match merged.process_events.as_mut() {
                    Some(existing) => existing.pages.extend(evts.pages),
                    None => merged.process_events = Some(evts),
                }
            }
            matched += 1;
        }

        // Audit findings #36/#37: "no handler matched" is normal — no PM
        // subscribed to this event type. Log at info-level and return the
        // empty merged response instead of erroring. Matches Python's
        // silent return in `dispatch_process_manager`.
        if matched == 0 {
            tracing::info!(
                type_url = %type_url,
                "no process-manager handler registered for event type — returning empty response"
            );
        }

        Ok(merged)
    }
}

fn extract_pm_event_type_url(request: &ProcessManagerHandleRequest) -> Result<String, ClientError> {
    use crate::convert::TYPE_URL_PREFIX;
    use crate::error_codes::{codes, keys, messages};

    let book = request.trigger.as_ref().ok_or_else(|| {
        ClientError::invalid_argument(
            codes::MISSING_PM_TRIGGER,
            messages::MISSING_PM_TRIGGER,
            std::iter::empty::<(String, String)>(),
        )
    })?;
    let page = book.pages.last().ok_or_else(|| {
        ClientError::invalid_argument(
            codes::EMPTY_PM_TRIGGER,
            messages::EMPTY_PM_TRIGGER,
            std::iter::empty::<(String, String)>(),
        )
    })?;
    let payload = match &page.payload {
        Some(crate::proto::event_page::Payload::Event(e)) => e,
        _ => {
            return Err(ClientError::invalid_argument(
                codes::MISSING_PM_EVENT_PAYLOAD,
                messages::MISSING_PM_EVENT_PAYLOAD,
                std::iter::empty::<(String, String)>(),
            ));
        }
    };
    // Audit finding #37: explicit-over-tolerant 4th check for parity with
    // saga (`extract_saga_event_type_url`). Empty or non-googleapis type
    // URLs raise INVALID_ARGUMENT instead of falling through to the
    // generic "no handler registered" path.
    if payload.type_url.is_empty() || !payload.type_url.starts_with(TYPE_URL_PREFIX) {
        return Err(ClientError::invalid_argument(
            codes::PM_INVALID_TYPE_URL,
            messages::PM_INVALID_TYPE_URL,
            [(keys::TYPE_URL, payload.type_url.clone())],
        ));
    }
    Ok(payload.type_url.clone())
}

/// Runtime router built from one-or-more projector factories.
#[derive(Debug)]
pub struct ProjectorRouter {
    pub(crate) factories: Vec<Factory>,
    /// Cached projector `name` (audit #42).
    pub(crate) cached_name: OnceLock<String>,
}

impl ProjectorRouter {
    /// Dispatch an `EventBook` through every registered projector.
    ///
    /// Audit finding #52: page-level outer loop, projector inner loop —
    /// matches Python's `dispatch_projector` semantics
    /// (`dispatch.py:608-640`). Each matching projector is instantiated
    /// once and reused across pages. Handler responses are explicitly
    /// ignored: projectors are side-effect-only, the framework returns
    /// a synthetic `Projection` carrying the book's cover and
    /// next_sequence regardless of how many projectors registered.
    ///
    /// Two consequences vs the previous "per-projector with full book"
    /// model:
    ///   1. Hand-rolled `Handler::dispatch` projectors that don't
    ///      iterate `book.pages` internally now still see every page
    ///      (the framework iterates).
    ///   2. Multi-projector responses no longer privilege the last
    ///      projector's `Projection` — every projector contributes its
    ///      side effects, none gets to dictate the response payload.
    pub fn dispatch(&self, book: EventBook) -> Result<Projection, ClientError> {
        // Only filter by domain when the book explicitly carries a cover.
        // Coverless books (used by tests that don't assert domain scoping)
        // pass through to every projector unchanged.
        let incoming_domain = book.cover.as_ref().map(|c| c.domain.clone());
        let cover = book.cover.clone();
        let next_sequence = book.next_sequence;

        // Instantiate each matching projector once and hold across the
        // page loop. Mirrors Python's
        // `[cls() for cls, factory in self._factories if domain matches]`.
        let mut matched: Vec<Box<dyn Handler>> = Vec::new();
        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            let declared_domains = match handler.config() {
                HandlerConfig::Projector { domains, .. } => domains,
                _ => continue,
            };
            // Wildcard "*" matches any.
            if let Some(ref d) = incoming_domain {
                let matches = declared_domains.iter().any(|x| x == d || x == "*");
                if !matches {
                    continue;
                }
            }
            matched.push(handler);
        }

        // Per-page outer loop. Each projector sees one page at a time
        // via a single-page book; the framework drives the iteration so
        // hand-rolled Handler::dispatch implementations don't silently
        // miss pages 2..N.
        for page in book.pages.iter() {
            let page_book = EventBook {
                cover: cover.clone(),
                pages: vec![page.clone()],
                next_sequence,
                ..book.clone()
            };
            for handler in &matched {
                let response = handler.dispatch(HandlerRequest::Projector(page_book.clone()))?;
                // Validate response variant but DROP the payload —
                // projectors are side-effect-only.
                let HandlerResponse::Projector(_) = response else {
                    return Err(ClientError::invalid_argument(
                        crate::error_codes::codes::HANDLER_WRONG_RESPONSE_KIND,
                        crate::error_codes::messages::HANDLER_WRONG_RESPONSE_KIND,
                        [(crate::error_codes::keys::EXPECTED_KIND, "Projector")],
                    ));
                };
            }
        }

        // Synthetic response. Always carries the book's cover and
        // next_sequence; `projector` left empty since no single
        // projector "owns" the response anymore.
        Ok(Projection {
            cover,
            projector: String::new(),
            sequence: next_sequence,
            projection: None,
        })
    }
}

macro_rules! impl_handler_count {
    ($ty:ty) => {
        impl $ty {
            /// Number of factories registered on this runtime router.
            pub fn handler_count(&self) -> usize {
                self.factories.len()
            }
        }
    };
}

impl_handler_count!(CommandHandlerRouter);
impl_handler_count!(SagaRouter);
impl_handler_count!(ProcessManagerRouter);
impl_handler_count!(ProjectorRouter);

/// Extract the first handler's [`HandlerConfig`] by invoking its factory once.
fn first_config(factories: &[Factory]) -> Option<HandlerConfig> {
    factories.first().map(|f| (f.produce)().config())
}

impl CommandHandlerRouter {
    /// Domain this router serves (read from the first registered handler's
    /// `#[command_handler(domain = ...)]` metadata).
    ///
    /// Audit #42: cached after the first call.
    pub fn name(&self) -> String {
        self.cached_name
            .get_or_init(|| match first_config(&self.factories) {
                Some(HandlerConfig::CommandHandler { domain, .. }) => domain,
                _ => String::new(),
            })
            .clone()
    }

    /// Command handlers don't emit cross-domain commands at the framework
    /// level — they return events. Always empty.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }

    /// Audit #45: True if any registered handler declares at least one
    /// `#[handles_fact]` method. The gRPC adapter consults this as the
    /// gate for the `HandleFact` RPC — false → return UNIMPLEMENTED
    /// without entering dispatch.
    ///
    /// Pure metadata read on the registered factories' configs; cheap
    /// enough to call per-request.
    pub fn supports_handle_fact(&self) -> bool {
        for factory in &self.factories {
            if let HandlerConfig::CommandHandler { handles_fact, .. } = (factory.produce)().config()
            {
                if !handles_fact.is_empty() {
                    return true;
                }
            }
        }
        false
    }

    /// Audit #45: True if any registered handler opted in via
    /// `#[command_handler(supports_replay = true)]`. The gRPC adapter
    /// uses this as the gate for the `Replay` RPC.
    pub fn supports_replay(&self) -> bool {
        for factory in &self.factories {
            if let HandlerConfig::CommandHandler {
                supports_replay, ..
            } = (factory.produce)().config()
            {
                if supports_replay {
                    return true;
                }
            }
        }
        false
    }

    /// Audit #45: dispatch a `FactRequest` through the registered
    /// command handler's `#[handles_fact]` methods.
    ///
    /// The single-handler invariant from audit #62 means at most one
    /// factory matches; this routes the whole request into that
    /// handler's `__angzarr_dispatch_fact` (emitted by the
    /// `#[command_handler]` macro) and unwraps the
    /// `HandlerResponse::HandleFact` variant.
    ///
    /// Caller (the gRPC adapter) should gate via
    /// [`Self::supports_handle_fact`] first.
    pub fn dispatch_fact(
        &self,
        request: crate::proto::FactRequest,
    ) -> Result<crate::proto::EventBook, ClientError> {
        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            // Only command-handler factories know how to dispatch facts.
            if !matches!(handler.config(), HandlerConfig::CommandHandler { .. }) {
                continue;
            }
            let response = handler.dispatch(HandlerRequest::HandleFact(request))?;
            return match response {
                HandlerResponse::HandleFact(book) => Ok(book),
                _ => Err(ClientError::invalid_argument(
                    crate::error_codes::codes::HANDLER_WRONG_RESPONSE_KIND,
                    crate::error_codes::messages::HANDLER_WRONG_RESPONSE_KIND,
                    [(crate::error_codes::keys::EXPECTED_KIND, "HandleFact")],
                )),
            };
        }
        // No CommandHandler factories — empty book matches the
        // "framework declined, coordinator handles fallback" semantic.
        Ok(crate::proto::EventBook::default())
    }

    /// Audit #45: dispatch a `ReplayRequest` through the registered
    /// command handler.
    pub fn dispatch_replay(
        &self,
        request: crate::proto::ReplayRequest,
    ) -> Result<crate::proto::ReplayResponse, ClientError> {
        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            if !matches!(handler.config(), HandlerConfig::CommandHandler { .. }) {
                continue;
            }
            let response = handler.dispatch(HandlerRequest::Replay(request))?;
            return match response {
                HandlerResponse::Replay(resp) => Ok(resp),
                _ => Err(ClientError::invalid_argument(
                    crate::error_codes::codes::HANDLER_WRONG_RESPONSE_KIND,
                    crate::error_codes::messages::HANDLER_WRONG_RESPONSE_KIND,
                    [(crate::error_codes::keys::EXPECTED_KIND, "Replay")],
                )),
            };
        }
        Ok(crate::proto::ReplayResponse::default())
    }
}

impl SagaRouter {
    /// Saga name (`#[saga(name = ...)]` from the first registered handler).
    /// Audit #42: cached after the first call.
    pub fn name(&self) -> String {
        self.cached_name
            .get_or_init(|| match first_config(&self.factories) {
                Some(HandlerConfig::Saga { name, .. }) => name,
                _ => String::new(),
            })
            .clone()
    }

    /// Output target domains from every registered saga's `#[saga(target = ...)]`,
    /// deduplicated.
    pub fn output_domains(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for factory in &self.factories {
            if let HandlerConfig::Saga { target, .. } = (factory.produce)().config() {
                if !target.is_empty() && !seen.contains(&target) {
                    seen.push(target);
                }
            }
        }
        seen
    }

    /// Audit #74: subset of [`output_domains`] that the registered
    /// sagas ever address with sync mode (`#[saga(sync = true)]`).
    /// Drives readiness probing — only sync targets need their
    /// coordinator reachable for traffic to be safe.
    pub fn sync_output_domains(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for factory in &self.factories {
            if let HandlerConfig::Saga { target, sync, .. } = (factory.produce)().config() {
                if sync && !target.is_empty() && !seen.contains(&target) {
                    seen.push(target);
                }
            }
        }
        seen
    }

    /// Audit #74: `true` if any registered saga has at least one async
    /// target — i.e. ever publishes through the async bus rather than
    /// calling the downstream coordinator synchronously. Per-handler
    /// check; the deduped set difference would miss the case of two
    /// handlers in the same router emitting to the same target with
    /// different sync flags.
    pub fn has_async_outputs(&self) -> bool {
        self.factories.iter().any(|factory| {
            matches!(
                (factory.produce)().config(),
                HandlerConfig::Saga {
                    sync: false,
                    ref target,
                    ..
                } if !target.is_empty()
            )
        })
    }
}

impl ProcessManagerRouter {
    /// Process-manager name (`#[process_manager(name = ...)]` from the first handler).
    /// Audit #42: cached after the first call.
    pub fn name(&self) -> String {
        self.cached_name
            .get_or_init(|| match first_config(&self.factories) {
                Some(HandlerConfig::ProcessManager { name, .. }) => name,
                _ => String::new(),
            })
            .clone()
    }

    /// Flattened, deduplicated `targets` across every registered PM.
    pub fn output_domains(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for factory in &self.factories {
            if let HandlerConfig::ProcessManager { targets, .. } = (factory.produce)().config() {
                for t in targets {
                    if !t.is_empty() && !seen.contains(&t) {
                        seen.push(t);
                    }
                }
            }
        }
        seen
    }

    /// Audit #74: flattened, deduplicated `sync_targets` across every
    /// registered PM — the subset of [`output_domains`] that ever uses
    /// sync mode.
    pub fn sync_output_domains(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for factory in &self.factories {
            if let HandlerConfig::ProcessManager { sync_targets, .. } = (factory.produce)().config()
            {
                for t in sync_targets {
                    if !t.is_empty() && !seen.contains(&t) {
                        seen.push(t);
                    }
                }
            }
        }
        seen
    }

    /// Audit #74: `true` if any registered PM has at least one target
    /// that's not in its own `sync_targets` — i.e. ever publishes
    /// through the async bus.
    pub fn has_async_outputs(&self) -> bool {
        self.factories.iter().any(|factory| {
            if let HandlerConfig::ProcessManager {
                targets,
                sync_targets,
                ..
            } = (factory.produce)().config()
            {
                targets
                    .iter()
                    .any(|t| !t.is_empty() && !sync_targets.contains(t))
            } else {
                false
            }
        })
    }
}

impl ProjectorRouter {
    /// Projector name (`#[projector(name = ...)]` from the first handler).
    /// Audit #42: cached after the first call.
    pub fn name(&self) -> String {
        self.cached_name
            .get_or_init(|| match first_config(&self.factories) {
                Some(HandlerConfig::Projector { name, .. }) => name,
                _ => String::new(),
            })
            .clone()
    }

    /// Projectors are read-side; no outbound destinations.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{event_page, EventBook, EventPage, PageHeader};
    use prost_types::Any as ProtoAny;

    fn saga_request_with_event(type_url: &str) -> SagaHandleRequest {
        SagaHandleRequest {
            source: Some(EventBook {
                pages: vec![EventPage {
                    header: Some(PageHeader::default()),
                    payload: Some(event_page::Payload::Event(ProtoAny {
                        type_url: type_url.to_string(),
                        value: vec![],
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Audit finding #35: P2.5 explicit-over-tolerant — Rust now rejects
    /// invalid type URLs in saga triggers, matching Python's four-case
    /// validation in `dispatch_saga`.
    #[test]
    fn extract_saga_rejects_empty_type_url() {
        let req = saga_request_with_event("");
        let err = extract_saga_event_type_url(&req).expect_err("empty type_url must error");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid type_url"),
            "expected message to mention 'invalid type_url', got: {msg}"
        );
        assert!(matches!(err, ClientError::InvalidArgument(_)));
    }

    #[test]
    fn extract_saga_rejects_non_googleapis_prefix() {
        let req = saga_request_with_event("type.example.com/foo.Bar");
        let err = extract_saga_event_type_url(&req).expect_err("non-googleapis prefix must error");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid type_url"),
            "expected message to mention 'invalid type_url', got: {msg}"
        );
    }

    #[test]
    fn extract_saga_accepts_valid_googleapis_url() {
        let req = saga_request_with_event("type.googleapis.com/examples.OrderCreated");
        let url = extract_saga_event_type_url(&req).expect("valid type_url must succeed");
        assert_eq!(url, "type.googleapis.com/examples.OrderCreated");
    }

    #[test]
    fn extract_saga_missing_source_message() {
        let req = SagaHandleRequest::default();
        let err = extract_saga_event_type_url(&req).expect_err("must error");
        assert!(err.to_string().contains("missing saga source"));
    }

    #[test]
    fn extract_saga_empty_source_message() {
        let req = SagaHandleRequest {
            source: Some(EventBook::default()),
            ..Default::default()
        };
        let err = extract_saga_event_type_url(&req).expect_err("must error");
        assert!(err.to_string().contains("empty saga source"));
    }

    #[test]
    fn extract_saga_missing_event_payload_message() {
        let req = SagaHandleRequest {
            source: Some(EventBook {
                pages: vec![EventPage {
                    header: Some(PageHeader::default()),
                    payload: None,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let err = extract_saga_event_type_url(&req).expect_err("must error");
        assert!(err.to_string().contains("missing event payload"));
    }

    // ----- PM trigger malformed-input parity (audit finding #37) -----

    fn pm_request_with_event(type_url: &str) -> ProcessManagerHandleRequest {
        ProcessManagerHandleRequest {
            trigger: Some(EventBook {
                pages: vec![EventPage {
                    header: Some(PageHeader::default()),
                    payload: Some(event_page::Payload::Event(ProtoAny {
                        type_url: type_url.to_string(),
                        value: vec![],
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn extract_pm_rejects_empty_type_url() {
        let req = pm_request_with_event("");
        let err = extract_pm_event_type_url(&req).expect_err("empty type_url must error");
        assert!(err.to_string().contains("invalid type_url"));
    }

    #[test]
    fn extract_pm_rejects_non_googleapis_prefix() {
        let req = pm_request_with_event("type.example.com/foo.Bar");
        let err = extract_pm_event_type_url(&req).expect_err("non-googleapis prefix must error");
        assert!(err.to_string().contains("invalid type_url"));
    }

    #[test]
    fn extract_pm_accepts_valid_googleapis_url() {
        let req = pm_request_with_event("type.googleapis.com/examples.OrderCreated");
        let url = extract_pm_event_type_url(&req).expect("valid type_url must succeed");
        assert_eq!(url, "type.googleapis.com/examples.OrderCreated");
    }

    #[test]
    fn extract_pm_missing_trigger_message() {
        let req = ProcessManagerHandleRequest::default();
        let err = extract_pm_event_type_url(&req).expect_err("must error");
        assert!(err.to_string().contains("missing PM trigger"));
    }

    #[test]
    fn extract_pm_empty_trigger_message() {
        let req = ProcessManagerHandleRequest {
            trigger: Some(EventBook::default()),
            ..Default::default()
        };
        let err = extract_pm_event_type_url(&req).expect_err("must error");
        assert!(err.to_string().contains("empty PM trigger"));
    }

    #[test]
    fn extract_pm_missing_event_payload_message() {
        let req = ProcessManagerHandleRequest {
            trigger: Some(EventBook {
                pages: vec![EventPage {
                    header: Some(PageHeader::default()),
                    payload: None,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let err = extract_pm_event_type_url(&req).expect_err("must error");
        assert!(err
            .to_string()
            .contains("missing event payload on PM trigger"));
    }
}
