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
    business_response, BusinessResponse, ContextualCommand, EventBook, Notification,
    ProcessManagerHandleRequest, ProcessManagerHandleResponse, Projection, RejectionNotification,
    SagaHandleRequest, SagaResponse,
};
use crate::router::builder::Factory;
use crate::router::{Handler, HandlerConfig, HandlerRequest, HandlerResponse};
use crate::ClientError;
use prost::Message;

/// Runtime router built from one-or-more aggregate factories.
#[derive(Debug)]
pub struct CommandHandlerRouter {
    pub(crate) factories: Vec<Factory>,
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

        // Threaded sequence — starts at the prior-events' next_sequence and
        // advances by each handler's emitted page count.
        let initial_next_seq = cmd.events.as_ref().map(|eb| eb.next_sequence).unwrap_or(0);
        let mut running_seq = initial_next_seq;

        let mut merged = EventBook {
            next_sequence: initial_next_seq,
            ..Default::default()
        };
        let mut matched = 0u32;

        for factory in &self.factories {
            // One factory call per factory: the instance serves both the
            // config peek and (if matched) the dispatch call.
            let handler: Box<dyn Handler> = (factory.produce)();
            let handles = match handler.config() {
                HandlerConfig::CommandHandler { handled, .. } => handled,
                _ => continue,
            };
            if !handles.iter().any(|u| u == &type_url) {
                continue;
            }

            // Hand each successive handler a ContextualCommand whose
            // events.next_sequence reflects prior emissions from earlier
            // handlers in the merged stream.
            let mut scoped_cmd = cmd.clone();
            if let Some(eb) = scoped_cmd.events.as_mut() {
                eb.next_sequence = running_seq;
            }

            let response = handler.dispatch(HandlerRequest::CommandHandler(scoped_cmd))?;
            let HandlerResponse::CommandHandler(br) = response else {
                return Err(ClientError::InvalidArgument(
                    "handler returned non-CommandHandler response".into(),
                ));
            };
            if let Some(business_response::Result::Events(events)) = br.result {
                running_seq += events.pages.len() as u32;
                merged.pages.extend(events.pages);
                if merged.cover.is_none() {
                    merged.cover = events.cover;
                }
            }
            matched += 1;
        }

        if matched == 0 {
            // Mirror Python's wording (`dispatch.py:246-249`): include
            // both domain and type_url so the user can distinguish
            // "wrong domain" from "wrong command type". P2.6 / audit
            // finding #11.
            let domain = cmd
                .command
                .as_ref()
                .and_then(|cb| cb.cover.as_ref())
                .map(|c| c.domain.as_str())
                .unwrap_or("<missing>");
            return Err(ClientError::InvalidArgument(format!(
                "no handler registered for domain={:?} type_url={:?}",
                domain, type_url
            )));
        }

        merged.next_sequence = running_seq;
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(merged)),
        })
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
                return Err(ClientError::InvalidArgument(
                    "handler returned non-CommandHandler response".into(),
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
    let book = cmd
        .command
        .as_ref()
        .ok_or_else(|| ClientError::InvalidArgument("missing command book".into()))?;
    let page = book
        .pages
        .first()
        .ok_or_else(|| ClientError::InvalidArgument("missing command page".into()))?;
    let payload = match &page.payload {
        Some(crate::proto::command_page::Payload::Command(c)) => c,
        _ => {
            return Err(ClientError::InvalidArgument(
                "missing command payload".into(),
            ));
        }
    };
    let notif = Notification::decode(payload.value.as_slice()).map_err(|e| {
        ClientError::InvalidArgument(format!("failed to decode Notification: {}", e))
    })?;
    let rejection = match notif.payload.as_ref() {
        Some(p) => RejectionNotification::decode(p.value.as_slice()).map_err(|e| {
            ClientError::InvalidArgument(format!("failed to decode RejectionNotification: {}", e))
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
    let book = cmd
        .command
        .as_ref()
        .ok_or_else(|| ClientError::InvalidArgument("missing command book".into()))?;
    let page = book
        .pages
        .first()
        .ok_or_else(|| ClientError::InvalidArgument("missing command page".into()))?;
    let payload = match &page.payload {
        Some(crate::proto::command_page::Payload::Command(c)) => c,
        _ => {
            return Err(ClientError::InvalidArgument(
                "missing command payload".into(),
            ));
        }
    };
    Ok(payload.type_url.clone())
}

/// Runtime router built from one-or-more saga factories.
#[derive(Debug)]
pub struct SagaRouter {
    pub(crate) factories: Vec<Factory>,
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

        let mut merged = SagaResponse::default();
        let mut matched = 0u32;

        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            let handles = match handler.config() {
                HandlerConfig::Saga { handled, .. } => handled,
                _ => continue,
            };
            if !handles.iter().any(|u| u == &type_url) {
                continue;
            }

            let response = handler.dispatch(HandlerRequest::Saga(request.clone()))?;
            let HandlerResponse::Saga(sr) = response else {
                return Err(ClientError::InvalidArgument(
                    "handler returned non-Saga response".into(),
                ));
            };
            merged.commands.extend(sr.commands);
            merged.events.extend(sr.events);
            matched += 1;
        }

        if matched == 0 {
            return Err(ClientError::InvalidArgument(format!(
                "no saga handler registered for event type: {}",
                type_url
            )));
        }

        Ok(merged)
    }
}

fn extract_saga_event_type_url(request: &SagaHandleRequest) -> Result<String, ClientError> {
    let book = request
        .source
        .as_ref()
        .ok_or_else(|| ClientError::InvalidArgument("missing saga source".into()))?;
    let page = book
        .pages
        .last()
        .ok_or_else(|| ClientError::InvalidArgument("empty saga source".into()))?;
    let payload = match &page.payload {
        Some(crate::proto::event_page::Payload::Event(e)) => e,
        _ => {
            return Err(ClientError::InvalidArgument("missing event payload".into()));
        }
    };
    Ok(payload.type_url.clone())
}

/// Runtime router built from one-or-more process-manager factories.
#[derive(Debug)]
pub struct ProcessManagerRouter {
    pub(crate) factories: Vec<Factory>,
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

        let mut merged = ProcessManagerHandleResponse::default();
        let mut matched = 0u32;

        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            let handles = match handler.config() {
                HandlerConfig::ProcessManager { handled, .. } => handled,
                _ => continue,
            };
            if !handles.iter().any(|u| u == &type_url) {
                continue;
            }

            let response = handler.dispatch(HandlerRequest::ProcessManager(request.clone()))?;
            let HandlerResponse::ProcessManager(pr) = response else {
                return Err(ClientError::InvalidArgument(
                    "handler returned non-ProcessManager response".into(),
                ));
            };
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

        if matched == 0 {
            return Err(ClientError::InvalidArgument(format!(
                "no process-manager handler registered for event type: {}",
                type_url
            )));
        }

        Ok(merged)
    }
}

fn extract_pm_event_type_url(request: &ProcessManagerHandleRequest) -> Result<String, ClientError> {
    let book = request
        .trigger
        .as_ref()
        .ok_or_else(|| ClientError::InvalidArgument("missing PM trigger".into()))?;
    let page = book
        .pages
        .last()
        .ok_or_else(|| ClientError::InvalidArgument("empty PM trigger".into()))?;
    let payload = match &page.payload {
        Some(crate::proto::event_page::Payload::Event(e)) => e,
        _ => {
            return Err(ClientError::InvalidArgument(
                "missing event payload on PM trigger".into(),
            ));
        }
    };
    Ok(payload.type_url.clone())
}

/// Runtime router built from one-or-more projector factories.
#[derive(Debug)]
pub struct ProjectorRouter {
    pub(crate) factories: Vec<Factory>,
}

impl ProjectorRouter {
    /// Dispatch an `EventBook` through every registered projector.
    ///
    /// Projector semantics differ from the other kinds: each projector's
    /// `Handler::dispatch` iterates every page in the book against its single
    /// instance, so projectors can batch side effects. The runtime does not
    /// merge handler outputs; it returns a skeleton `Projection` carrying the
    /// book's cover and the last registered projector's name.
    pub fn dispatch(&self, book: EventBook) -> Result<Projection, ClientError> {
        let mut last_projection: Option<Projection> = None;
        // Only filter by domain when the book explicitly carries a cover.
        // Coverless books (used by tests that don't assert domain scoping)
        // pass through to every projector unchanged.
        let incoming_domain = book.cover.as_ref().map(|c| c.domain.clone());

        for factory in &self.factories {
            let handler: Box<dyn Handler> = (factory.produce)();
            let declared_domains = match handler.config() {
                HandlerConfig::Projector { domains, .. } => domains,
                _ => continue,
            };

            // Skip projectors whose declared domains don't cover the
            // incoming book's domain. Wildcard "*" matches any.
            if let Some(ref d) = incoming_domain {
                let matches = declared_domains.iter().any(|x| x == d || x == "*");
                if !matches {
                    continue;
                }
            }

            let response = handler.dispatch(HandlerRequest::Projector(book.clone()))?;
            let HandlerResponse::Projector(pr) = response else {
                return Err(ClientError::InvalidArgument(
                    "handler returned non-Projector response".into(),
                ));
            };
            last_projection = Some(pr);
        }

        // If no projectors registered, return an empty Projection carrying
        // the book's cover so callers still get a valid proto back.
        Ok(last_projection.unwrap_or(Projection {
            cover: book.cover,
            projector: String::new(),
            sequence: book.next_sequence,
            projection: None,
        }))
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
    pub fn name(&self) -> String {
        match first_config(&self.factories) {
            Some(HandlerConfig::CommandHandler { domain, .. }) => domain,
            _ => String::new(),
        }
    }

    /// Command handlers don't emit cross-domain commands at the framework
    /// level — they return events. Always empty.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }
}

impl SagaRouter {
    /// Saga name (`#[saga(name = ...)]` from the first registered handler).
    pub fn name(&self) -> String {
        match first_config(&self.factories) {
            Some(HandlerConfig::Saga { name, .. }) => name,
            _ => String::new(),
        }
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
}

impl ProcessManagerRouter {
    /// Process-manager name (`#[process_manager(name = ...)]` from the first handler).
    pub fn name(&self) -> String {
        match first_config(&self.factories) {
            Some(HandlerConfig::ProcessManager { name, .. }) => name,
            _ => String::new(),
        }
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
}

impl ProjectorRouter {
    /// Projector name (`#[projector(name = ...)]` from the first handler).
    pub fn name(&self) -> String {
        match first_config(&self.factories) {
            Some(HandlerConfig::Projector { name, .. }) => name,
            _ => String::new(),
        }
    }

    /// Projectors are read-side; no outbound destinations.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }
}
