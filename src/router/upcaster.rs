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

use crate::error::ClientError;
use crate::proto::{event_page, EventPage, UpcastRequest, UpcastResponse};
use crate::router::builder::Factory;
use crate::router::handler::{Handler, HandlerConfig, HandlerRequest, HandlerResponse, Kind};

/// Runtime router dispatching upcast requests through registered upcaster handlers.
pub struct UpcasterRouter {
    pub(crate) factories: Vec<Factory>,
    /// Cached upcaster `name` (audit #42).
    pub(crate) cached_name: std::sync::OnceLock<String>,
}

impl UpcasterRouter {
    /// Upcaster name (`#[upcaster(name = ...)]` from the first registered handler).
    /// Audit #42: cached after the first call.
    pub fn name(&self) -> String {
        self.cached_name
            .get_or_init(|| match self.factories.first().map(|f| f.config()) {
                Some(HandlerConfig::Upcaster { name, .. }) => name.clone(),
                _ => String::new(),
            })
            .clone()
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
                // Only dispatch to upcasters whose domain matches —
                // metadata read from the cached config so a domain
                // mismatch doesn't materialize a handler instance.
                let matches_domain = matches!(
                    factory.config(),
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
                let handler: Box<dyn Handler> = (factory.produce)();
                let response = handler.dispatch(HandlerRequest::Upcaster(single))?;
                let HandlerResponse::Upcaster(r) = response else {
                    return Err(ClientError::invalid_argument(
                        crate::error_codes::codes::UPCASTER_WRONG_RESPONSE_KIND,
                        crate::error_codes::messages::UPCASTER_WRONG_RESPONSE_KIND,
                        [(crate::error_codes::keys::EXPECTED_KIND, Kind::Upcaster.as_str())],
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
