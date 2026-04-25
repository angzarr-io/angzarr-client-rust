//! Factory-based runtime router for upcasters (event-version transforms).
//!
//! `UpcasterRouter` holds the same kind of factory list as the other
//! runtime routers — one entry per `#[upcaster]` class registered with
//! [`crate::router::Router`]. Each factory produces a fresh `Box<dyn Handler>`
//! per dispatch so handlers stay stateless across requests.
//!
//! Dispatch walks each `EventPage` in the incoming `UpcastRequest.events`.
//! For each page with an embedded event, every registered handler whose
//! `HandlerConfig::Upcaster.domain` matches `request.domain` is invoked in
//! registration order; the first handler whose `#[upcasts]` pair's
//! `from_type_url` matches the event's `type_url` transforms it. Pages
//! without a matching upcast (or without an event payload) pass through
//! unchanged.
//!
//! The previous `UpcasterRouter::new(domain).on(suffix, handler)` API has
//! been removed in favor of the unified `Router::new(name).with_handler(…)`
//! flow.

use std::sync::Arc;

use crate::error::ClientError;
use crate::proto::{event_page, EventPage, UpcastRequest, UpcastResponse};
use crate::router::builder::Factory;
use crate::router::handler::{Handler, HandlerConfig, HandlerRequest, HandlerResponse};

/// Runtime router dispatching upcast requests through registered upcaster handlers.
pub struct UpcasterRouter {
    pub(crate) factories: Vec<Factory>,
}

impl UpcasterRouter {
    /// Upcaster name (`#[upcaster(name = ...)]` from the first registered handler).
    pub fn name(&self) -> String {
        match self.factories.first().map(|f| (f.produce)().config()) {
            Some(HandlerConfig::Upcaster { name, .. }) => name,
            _ => String::new(),
        }
    }

    /// Upcasters are stream transformers; no outbound destinations.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }
}

impl std::fmt::Debug for UpcasterRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpcasterRouter")
            .field("handler_count", &self.factories.len())
            .finish()
    }
}

impl UpcasterRouter {
    /// Number of registered upcaster factories.
    pub fn handler_count(&self) -> usize {
        self.factories.len()
    }

    /// Dispatch an [`UpcastRequest`] through every registered upcaster.
    ///
    /// Events pass through every matching handler in registration order.
    /// The first handler whose `#[upcasts(from = …)]` matches the event's
    /// type URL (and whose `#[upcaster(domain = …)]` matches the request
    /// domain) transforms it; subsequent handlers see the transformed event.
    /// Events with no matching transform are returned unchanged.
    pub fn dispatch(&self, request: UpcastRequest) -> Result<UpcastResponse, ClientError> {
        let domain = request.domain.clone();
        let mut out: Vec<EventPage> = Vec::with_capacity(request.events.len());

        for page in request.events.iter() {
            // No event payload? pass through.
            let Some(event_page::Payload::Event(_)) = &page.payload else {
                out.push(page.clone());
                continue;
            };

            // Run the page through each factory in order; each handler can
            // transform the event further. The output of one upcaster is
            // the input of the next.
            let mut current_page = page.clone();

            for factory in &self.factories {
                let handler: Box<dyn Handler> = (factory.produce)();
                let cfg = handler.config();
                // Only dispatch to upcasters whose domain matches.
                let matches_domain = matches!(
                    &cfg,
                    HandlerConfig::Upcaster { domain: d, .. } if d == &domain
                );
                if !matches_domain {
                    continue;
                }

                // Drive the Handler with a single-event UpcastRequest so the
                // handler's generated dispatch runs its `#[upcasts]` routing
                // logic against just this page.
                let single = UpcastRequest {
                    domain: domain.clone(),
                    events: vec![current_page.clone()],
                };
                let response = handler.dispatch(HandlerRequest::Upcaster(single))?;
                let HandlerResponse::Upcaster(r) = response else {
                    return Err(ClientError::InvalidArgument(
                        "upcaster handler returned non-Upcaster response".into(),
                    ));
                };
                if let Some(transformed) = r.events.into_iter().next() {
                    current_page = transformed;
                }
            }

            out.push(current_page);
        }

        Ok(UpcastResponse { events: out })
    }
}

// ---------------------------------------------------------------------------
// Internal helper type used by the proc-macro expansion of `#[upcaster]` so
// generated code doesn't need to reach into `router::upcaster::UpcasterRouter`
// internals directly.
// ---------------------------------------------------------------------------

/// Opaque handle to a boxed upcaster function used in macro-generated dispatch
/// tables. Exported for `angzarr-macros` expansion; no public use expected.
#[doc(hidden)]
pub type UpcastFn = Arc<dyn Fn(&prost_types::Any) -> prost_types::Any + Send + Sync>;
