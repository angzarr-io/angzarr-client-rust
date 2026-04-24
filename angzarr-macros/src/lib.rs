//! Procedural macros for angzarr OO-style component definitions.
//!
//! # Aggregate Example
//!
//! ```rust,ignore
//! use angzarr_macros::{aggregate, handles, applies, rejected};
//!
//! #[aggregate(domain = "player")]
//! impl PlayerAggregate {
//!     type State = PlayerState;
//!
//!     #[applies(PlayerRegistered)]
//!     fn apply_registered(state: &mut PlayerState, event: PlayerRegistered) {
//!         state.player_id = format!("player_{}", event.email);
//!         state.display_name = event.display_name;
//!         state.exists = true;
//!     }
//!
//!     #[applies(FundsDeposited)]
//!     fn apply_deposited(state: &mut PlayerState, event: FundsDeposited) {
//!         if let Some(balance) = event.new_balance {
//!             state.bankroll = balance.amount;
//!         }
//!     }
//!
//!     #[handles(RegisterPlayer)]
//!     fn register(&self, cb: &CommandBook, cmd: RegisterPlayer, state: &PlayerState, seq: u32)
//!         -> CommandResult<EventBook> {
//!         // ...
//!     }
//!
//!     #[rejected(domain = "payment", command = "ProcessPayment")]
//!     fn handle_payment_rejected(&self, notification: &Notification, state: &PlayerState)
//!         -> CommandResult<BusinessResponse> {
//!         // ...
//!     }
//! }
//! ```
//!
//! # Saga Example
//!
//! ```rust,ignore
//! use angzarr_macros::{saga, handles};
//!
//! #[saga(name = "saga-order-fulfillment", source = "order", target = "inventory")]
//! impl OrderFulfillmentSaga {
//!     #[handles(OrderCompleted)]
//!     fn handle_completed(&self, event: OrderCompleted)
//!         -> CommandResult<SagaResponse> {
//!         // ...
//!     }
//! }
//! ```

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Attribute, Ident, ImplItem, ItemImpl, Meta, Token};

/// Kind attributes that may NOT coexist on the same impl.
const KIND_ATTRS: &[&str] = &["aggregate", "saga", "process_manager", "projector"];

/// If `attrs` contains a sibling kind attribute, return a `compile_error!` TokenStream.
///
/// Invoked at the top of each kind-macro entry point so stacking two kinds on
/// the same impl fails fast with a clear message, instead of surfacing as an
/// E0119 trait conflict against the macro-generated code later.
fn reject_stacked_kinds(this_kind: &str, attrs: &[Attribute]) -> Option<TokenStream2> {
    for attr in attrs {
        for kind in KIND_ATTRS {
            if attr.path().is_ident(kind) {
                let msg = format!(
                    "#[{this_kind}] cannot coexist with #[{kind}] on the same impl; exactly one of #[aggregate] / #[saga] / #[process_manager] / #[projector] is allowed"
                );
                return Some(quote! { ::std::compile_error!(#msg); });
            }
        }
    }
    None
}

/// Marks an impl block as an aggregate with command handlers.
///
/// # Attributes
/// - `domain = "name"` - The aggregate's domain name (required)
///
/// # Example
/// ```rust,ignore
/// #[aggregate(domain = "player")]
/// impl PlayerAggregate {
///     #[handles(RegisterPlayer)]
///     fn register(&self, cmd: RegisterPlayer, state: &PlayerState, seq: u32)
///         -> CommandResult<EventBook> {
///         // ...
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn aggregate(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_command_handler(attr, item, "aggregate")
}

/// Cross-language alias for `#[aggregate]`. Mirrors Python's
/// `@command_handler`. Currently both names produce identical code; prefer
/// `#[command_handler]` in new code — `#[aggregate]` stays pre-1.0 for
/// continuity but may be deprecated after CI is wired up.
#[proc_macro_attribute]
pub fn command_handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_command_handler(attr, item, "command_handler")
}

fn expand_command_handler(attr: TokenStream, item: TokenStream, kind_label: &str) -> TokenStream {
    let args = parse_macro_input!(attr as AggregateArgs);
    let input = parse_macro_input!(item as ItemImpl);

    if let Some(err) = reject_stacked_kinds(kind_label, &input.attrs) {
        return TokenStream::from(err);
    }

    let expanded = expand_aggregate(args, input);
    TokenStream::from(expanded)
}

struct AggregateArgs {
    domain: String,
    state: Ident,
}

impl syn::parse::Parse for AggregateArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut domain = None;
        let mut state = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match ident.to_string().as_str() {
                "domain" => {
                    let value: syn::LitStr = input.parse()?;
                    domain = Some(value.value());
                }
                "state" => {
                    let value: Ident = input.parse()?;
                    state = Some(value);
                }
                _ => return Err(syn::Error::new(ident.span(), "unknown attribute")),
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(AggregateArgs {
            domain: domain.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "domain is required")
            })?,
            state: state.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "state is required")
            })?,
        })
    }
}

fn expand_aggregate(args: AggregateArgs, mut input: ItemImpl) -> TokenStream2 {
    let domain = &args.domain;
    let state_ty = &args.state;
    let self_ty_for_applies = input.self_ty.clone();

    let meta = collect_method_metadata(&input);

    // Strip method-level marker attributes so rustc doesn't see them as unknown attrs.
    strip_method_markers(&mut input);

    let handled_exprs = meta
        .handled
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let applies_exprs = meta
        .applies
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let rejected_exprs = meta.rejected.iter().map(|(d, c)| {
        quote! { (#d.to_string(), #c.to_string()) }
    });
    let state_factory_expr = match &meta.state_factory {
        Some(name) => {
            let s = name.to_string();
            quote! { ::std::option::Option::Some(#s.to_string()) }
        }
        None => quote! { ::std::option::Option::None },
    };

    let dispatch_arms: Vec<TokenStream2> = meta
        .handled_with_methods
        .iter()
        .map(|(method_ident, cmd_ty)| {
            quote! {
                if *type_url == ::angzarr_client::full_type_url::<#cmd_ty>() {
                    let cmd_val = <#cmd_ty as ::prost::Message>::decode(payload.value.as_slice())
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(
                            ::std::format!("decode error for {}: {}", type_url, e)
                        ))?;
                    let events = self.#method_ident(cmd_val, &state, seq)
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(e.reason))?;
                    return ::std::result::Result::Ok(
                        ::angzarr_client::router::HandlerResponse::CommandHandler(
                            ::angzarr_client::proto::BusinessResponse {
                                result: ::std::option::Option::Some(
                                    ::angzarr_client::proto::business_response::Result::Events(events),
                                ),
                            },
                        ),
                    );
                }
            }
        })
        .collect();

    // `#[applies(E)]` replay arms: for each prior event whose type_url matches E,
    // decode and call the applier with `&mut state`.
    let apply_arms: Vec<TokenStream2> = meta
        .applies_with_methods
        .iter()
        .map(|(method_ident, evt_ty)| {
            quote! {
                if *evt_type_url == ::angzarr_client::full_type_url::<#evt_ty>() {
                    let evt_val = <#evt_ty as ::prost::Message>::decode(evt_any.value.as_slice())
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(
                            ::std::format!("decode error for {}: {}", evt_type_url, e)
                        ))?;
                    <#self_ty_for_applies>::#method_ident(&mut state, evt_val);
                    continue;
                }
            }
        })
        .collect();

    // Initial state constructor: `#[state_factory]` if declared, else `Default::default()`.
    let initial_state_expr = match &meta.state_factory {
        Some(method_ident) => {
            quote! { <#self_ty_for_applies>::#method_ident() }
        }
        None => {
            quote! { <#state_ty as ::std::default::Default>::default() }
        }
    };

    // `#[rejected(domain, command)]` arms. Each matches the rejection key
    // extracted from the incoming Notification's RejectionNotification payload.
    let rejection_arms: Vec<TokenStream2> = meta
        .rejected_with_methods
        .iter()
        .map(|(method_ident, rej_domain, rej_command)| {
            quote! {
                if target_domain == #rej_domain && target_command_suffix == #rej_command {
                    let response = self.#method_ident(&notification, &state)
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(e.reason))?;
                    return ::std::result::Result::Ok(
                        ::angzarr_client::router::HandlerResponse::CommandHandler(response),
                    );
                }
            }
        })
        .collect();

    let self_ty = &input.self_ty;
    quote! {
        #input

        impl ::angzarr_client::router::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::router::Kind =
                ::angzarr_client::router::Kind::CommandHandler;
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                ::angzarr_client::router::HandlerConfig::CommandHandler {
                    domain: #domain.to_string(),
                    handled: ::std::vec![#(#handled_exprs),*],
                    rejected: ::std::vec![#(#rejected_exprs),*],
                    applies: ::std::vec![#(#applies_exprs),*],
                    state_factory: #state_factory_expr,
                }
            }

            fn dispatch(
                &self,
                request: ::angzarr_client::router::HandlerRequest,
            ) -> ::std::result::Result<
                ::angzarr_client::router::HandlerResponse,
                ::angzarr_client::ClientError,
            > {
                let ctx_cmd = match request {
                    ::angzarr_client::router::HandlerRequest::CommandHandler(c) => c,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::InvalidArgument(
                                "aggregate dispatch requires HandlerRequest::CommandHandler".into(),
                            ),
                        );
                    }
                };

                let cmd_book = ctx_cmd.command.as_ref().ok_or_else(|| {
                    ::angzarr_client::ClientError::InvalidArgument("missing command book".into())
                })?;
                let cmd_page = cmd_book.pages.first().ok_or_else(|| {
                    ::angzarr_client::ClientError::InvalidArgument("missing command page".into())
                })?;
                let payload = match &cmd_page.payload {
                    ::std::option::Option::Some(
                        ::angzarr_client::proto::command_page::Payload::Command(c),
                    ) => c,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::InvalidArgument(
                                "missing command payload".into(),
                            ),
                        );
                    }
                };
                let type_url = &payload.type_url;

                let prior_events = ctx_cmd.events.clone().unwrap_or_default();
                let seq = ::angzarr_client::EventBookExt::next_sequence(&prior_events);

                // R7: construct initial state (via `#[state_factory]` if declared)
                // and replay prior events through `#[applies]` methods.
                let mut state: #state_ty = #initial_state_expr;
                for page in &prior_events.pages {
                    let evt_any = match &page.payload {
                        ::std::option::Option::Some(
                            ::angzarr_client::proto::event_page::Payload::Event(e),
                        ) => e,
                        _ => continue,
                    };
                    let evt_type_url = &evt_any.type_url;
                    #(#apply_arms)*
                    // No `#[applies]` match → skip silently (handlers may choose
                    // to ignore events they don't apply on).
                }

                // R10: Notification → rejection branch.
                if *type_url == ::angzarr_client::full_type_url::<
                    ::angzarr_client::proto::Notification
                >() {
                    let notification =
                        <::angzarr_client::proto::Notification as ::prost::Message>::decode(
                            payload.value.as_slice(),
                        )
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(
                            ::std::format!("failed to decode Notification: {}", e),
                        ))?;

                    let rejection = match notification.payload.as_ref() {
                        ::std::option::Option::Some(p) => {
                            <::angzarr_client::proto::RejectionNotification as ::prost::Message>::decode(
                                p.value.as_slice(),
                            )
                            .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(
                                ::std::format!("failed to decode RejectionNotification: {}", e),
                            ))?
                        }
                        ::std::option::Option::None =>
                            ::angzarr_client::proto::RejectionNotification::default(),
                    };

                    let target_domain = rejection
                        .rejected_command
                        .as_ref()
                        .and_then(|cb| cb.cover.as_ref().map(|c| c.domain.clone()))
                        .unwrap_or_default();
                    let target_command_suffix = rejection
                        .rejected_command
                        .as_ref()
                        .and_then(|cb| {
                            cb.pages.first().and_then(|p| match &p.payload {
                                ::std::option::Option::Some(
                                    ::angzarr_client::proto::command_page::Payload::Command(a),
                                ) => ::std::option::Option::Some(a.type_url.clone()),
                                _ => ::std::option::Option::None,
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
                    let _ = &seq; // seq unused on rejection path for R10

                    #(#rejection_arms)*

                    // No matching rejection handler → empty compensation.
                    return ::std::result::Result::Ok(
                        ::angzarr_client::router::HandlerResponse::CommandHandler(
                            ::angzarr_client::proto::BusinessResponse {
                                result: ::std::option::Option::Some(
                                    ::angzarr_client::proto::business_response::Result::Events(
                                        ::angzarr_client::proto::EventBook::default(),
                                    ),
                                ),
                            },
                        ),
                    );
                }

                #(#dispatch_arms)*

                ::std::result::Result::Err(
                    ::angzarr_client::ClientError::InvalidArgument(
                        ::std::format!("no handler for type: {}", type_url),
                    ),
                )
            }
        }
    }
}

/// Metadata harvested from method-level attribute markers inside a kind-impl.
struct MethodMetadata {
    /// Types named in `#[handles(T)]` (kept for config emission).
    handled: Vec<Ident>,
    /// `(method name, command type)` pairs for dispatch-arm generation.
    handled_with_methods: Vec<(Ident, Ident)>,
    /// `(domain, command)` pairs from `#[rejected(domain = "...", command = "...")]`.
    rejected: Vec<(String, String)>,
    /// `(method name, domain, command)` triples for rejection-arm generation.
    rejected_with_methods: Vec<(Ident, String, String)>,
    /// Types named in `#[applies(T)]` (kept for config emission).
    applies: Vec<Ident>,
    /// `(method name, event type)` pairs for state-rebuild-arm generation.
    applies_with_methods: Vec<(Ident, Ident)>,
    /// Name of the method annotated with `#[state_factory]`, if any.
    state_factory: Option<Ident>,
}

fn collect_method_metadata(input: &ItemImpl) -> MethodMetadata {
    let mut handled = Vec::new();
    let mut handled_with_methods = Vec::new();
    let mut rejected = Vec::new();
    let mut rejected_with_methods = Vec::new();
    let mut applies = Vec::new();
    let mut applies_with_methods = Vec::new();
    let mut state_factory = None;

    for item in &input.items {
        let ImplItem::Fn(method) = item else { continue };
        for attr in &method.attrs {
            if attr.path().is_ident("handles") {
                if let Ok(ty) = get_attr_ident(attr) {
                    handled.push(ty.clone());
                    handled_with_methods.push((method.sig.ident.clone(), ty));
                }
            } else if attr.path().is_ident("applies") {
                if let Ok(ty) = get_attr_ident(attr) {
                    applies.push(ty.clone());
                    applies_with_methods.push((method.sig.ident.clone(), ty));
                }
            } else if attr.path().is_ident("rejected") {
                if let Ok((d, c)) = get_rejected_args(attr) {
                    rejected.push((d.clone(), c.clone()));
                    rejected_with_methods.push((method.sig.ident.clone(), d, c));
                }
            } else if attr.path().is_ident("state_factory") {
                state_factory = Some(method.sig.ident.clone());
            }
        }
    }

    MethodMetadata {
        handled,
        handled_with_methods,
        rejected,
        rejected_with_methods,
        applies,
        applies_with_methods,
        state_factory,
    }
}

/// Strip `#[handles]`, `#[applies]`, `#[rejected]`, `#[state_factory]` from
/// the methods of an impl block so rustc doesn't see them as unknown attrs.
fn strip_method_markers(input: &mut ItemImpl) {
    for item in &mut input.items {
        if let ImplItem::Fn(method) = item {
            method.attrs.retain(|attr| {
                !attr.path().is_ident("handles")
                    && !attr.path().is_ident("rejected")
                    && !attr.path().is_ident("applies")
                    && !attr.path().is_ident("state_factory")
            });
        }
    }
}

/// Marks a method as a command handler.
///
/// # Example
/// ```rust,ignore
/// #[handles(RegisterPlayer)]
/// fn register(&self, cmd: RegisterPlayer, state: &PlayerState, seq: u32)
///     -> CommandResult<EventBook> {
///     // ...
/// }
/// ```
#[proc_macro_attribute]
pub fn handles(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // The actual work is done by the #[aggregate] macro
    // This is just a marker attribute
    item
}

/// Marks a method as a rejection handler.
///
/// # Attributes
/// - `domain = "name"` - The domain of the rejected command
/// - `command = "name"` - The type of the rejected command
///
/// # Example
/// ```rust,ignore
/// #[rejected(domain = "payment", command = "ProcessPayment")]
/// fn handle_payment_rejected(&self, notification: &Notification, state: &PlayerState)
///     -> CommandResult<BusinessResponse> {
///     // ...
/// }
/// ```
#[proc_macro_attribute]
pub fn rejected(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // The actual work is done by the #[aggregate] or #[process_manager] macro
    // This is just a marker attribute
    item
}

/// Marks a method as an event applier for state reconstruction.
///
/// The method must be a static function with signature:
/// `fn(state: &mut State, event: EventType)`
///
/// The #[aggregate] macro collects these and generates:
/// - `apply_event(state, event_any)` - dispatches to the right applier
/// - `rebuild(events)` - reconstructs state from event book
///
/// # Example
/// ```rust,ignore
/// #[applies(PlayerRegistered)]
/// fn apply_registered(state: &mut PlayerState, event: PlayerRegistered) {
///     state.player_id = format!("player_{}", event.email);
///     state.display_name = event.display_name;
///     state.exists = true;
/// }
///
/// #[applies(FundsDeposited)]
/// fn apply_deposited(state: &mut PlayerState, event: FundsDeposited) {
///     if let Some(balance) = event.new_balance {
///         state.bankroll = balance.amount;
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn applies(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // The actual work is done by the #[aggregate] macro
    // This is just a marker attribute
    item
}

/// Marks a static method as the factory producing an aggregate's initial
/// state. Cross-language alias for Python's `@state_factory` — the
/// `#[aggregate]` / `#[command_handler]` macro inspects it during codegen.
///
/// # Example
/// ```rust,ignore
/// #[state_factory]
/// fn empty() -> PlayerState { PlayerState::default() }
/// ```
#[proc_macro_attribute]
pub fn state_factory(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a method as an upcaster transforming one event type into another.
/// Cross-language alias for Python's `@upcasts`. The `#[upcaster]` macro
/// collects the (from_type, to_type) pair from the attribute arguments
/// and stamps it into the handler's `HandlerConfig`.
///
/// # Example
/// ```rust,ignore
/// #[upcasts(from = OldType, to = NewType)]
/// fn upcast(old: OldType) -> NewType { old.into() }
/// ```
#[proc_macro_attribute]
pub fn upcasts(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks an impl block as an upcaster. Cross-language alias for Python's
/// `@upcaster`. Currently a passthrough marker — the full Handler-trait
/// / UpcasterRouter integration is tracked under O-1b follow-up work.
///
/// # Attributes
/// - `name = "..."` - upcaster name
/// - `domain = "..."` - domain of the upcaster
#[proc_macro_attribute]
pub fn upcaster(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks an impl block as a saga with event handlers.
///
/// Sagas are pure translators: they receive source events and produce commands
/// with deferred sequences. The framework handles sequence assignment on delivery.
///
/// # Attributes
/// - `name = "saga-name"` - The saga's name (required)
/// - `source = "domain"` - Source domain whose events drive the saga (required)
/// - `target = "domain"` - Target domain that emitted commands/facts land on (required)
///
/// # Example
/// ```rust,ignore
/// #[saga(name = "saga-order-fulfillment", source = "order", target = "inventory")]
/// impl OrderFulfillmentSaga {
///     #[handles(OrderCompleted)]
///     fn handle_completed(&self, event: OrderCompleted) -> CommandResult<SagaResponse> {
///         // Build commands with cover set (framework stamps angzarr_deferred)
///         // ...
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn saga(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as SagaArgs);
    let input = parse_macro_input!(item as ItemImpl);

    if let Some(err) = reject_stacked_kinds("saga", &input.attrs) {
        return TokenStream::from(err);
    }

    let expanded = expand_saga(args, input);
    TokenStream::from(expanded)
}

struct SagaArgs {
    name: String,
    source: String,
    #[allow(dead_code)] // consumed once the saga macro is R1-ified in R11
    target: String,
}

impl syn::parse::Parse for SagaArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut source = None;
        let mut target = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: syn::LitStr = input.parse()?;

            match ident.to_string().as_str() {
                "name" => name = Some(value.value()),
                "source" => source = Some(value.value()),
                "target" => target = Some(value.value()),
                _ => return Err(syn::Error::new(ident.span(), "unknown attribute")),
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(SagaArgs {
            name: name.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "name is required")
            })?,
            source: source.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "source is required")
            })?,
            target: target.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "target is required")
            })?,
        })
    }
}

fn expand_saga(args: SagaArgs, mut input: ItemImpl) -> TokenStream2 {
    let name = &args.name;
    let source = &args.source;
    let target = &args.target;

    let meta = collect_method_metadata(&input);
    strip_method_markers(&mut input);

    let handled_exprs = meta
        .handled
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let rejected_exprs = meta.rejected.iter().map(|(d, c)| {
        quote! { (#d.to_string(), #c.to_string()) }
    });

    let dispatch_arms: Vec<TokenStream2> = meta
        .handled_with_methods
        .iter()
        .map(|(method_ident, evt_ty)| {
            quote! {
                if event_any.type_url == ::angzarr_client::full_type_url::<#evt_ty>() {
                    let evt = <#evt_ty as ::prost::Message>::decode(event_any.value.as_slice())
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(
                            ::std::format!("decode error for {}: {}", event_any.type_url, e)
                        ))?;
                    let response = self.#method_ident(evt)
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(e.reason))?;
                    return ::std::result::Result::Ok(
                        ::angzarr_client::router::HandlerResponse::Saga(response),
                    );
                }
            }
        })
        .collect();

    let self_ty = &input.self_ty;
    quote! {
        #input

        impl ::angzarr_client::router::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::router::Kind =
                ::angzarr_client::router::Kind::Saga;
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                ::angzarr_client::router::HandlerConfig::Saga {
                    name: #name.to_string(),
                    source: #source.to_string(),
                    target: #target.to_string(),
                    handled: ::std::vec![#(#handled_exprs),*],
                    rejected: ::std::vec![#(#rejected_exprs),*],
                }
            }

            fn dispatch(
                &self,
                request: ::angzarr_client::router::HandlerRequest,
            ) -> ::std::result::Result<
                ::angzarr_client::router::HandlerResponse,
                ::angzarr_client::ClientError,
            > {
                let saga_req = match request {
                    ::angzarr_client::router::HandlerRequest::Saga(r) => r,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::InvalidArgument(
                                "saga dispatch requires HandlerRequest::Saga".into(),
                            ),
                        );
                    }
                };

                let source_book = saga_req.source.as_ref().ok_or_else(|| {
                    ::angzarr_client::ClientError::InvalidArgument("missing saga source".into())
                })?;
                let event_page = source_book.pages.last().ok_or_else(|| {
                    ::angzarr_client::ClientError::InvalidArgument("empty saga source".into())
                })?;
                let event_any = match &event_page.payload {
                    ::std::option::Option::Some(
                        ::angzarr_client::proto::event_page::Payload::Event(e),
                    ) => e,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::InvalidArgument(
                                "missing event payload".into(),
                            ),
                        );
                    }
                };

                #(#dispatch_arms)*

                // No #[handles] match → empty SagaResponse (runtime merge handles it).
                ::std::result::Result::Ok(
                    ::angzarr_client::router::HandlerResponse::Saga(
                        ::angzarr_client::proto::SagaResponse::default(),
                    ),
                )
            }
        }
    }
}

/// Marks an impl block as a process manager with event handlers.
///
/// # Attributes
/// - `name = "pm-name"` - The PM's name (required)
/// - `domain = "pm-domain"` - The PM's own domain for state (required)
/// - `state = StateType` - The PM's state type (required)
/// - `inputs = ["domain1", "domain2"]` - Input domains to subscribe to (required)
///
/// # Example
/// ```rust,ignore
/// #[process_manager(name = "hand-flow", domain = "hand-flow", state = PMState, inputs = ["table", "hand"])]
/// impl HandFlowPM {
///     #[applies(PMStateUpdated)]
///     fn apply_state(state: &mut PMState, event: PMStateUpdated) {
///         // ...
///     }
///
///     #[handles(HandStarted)]
///     fn handle_hand(&self, event: HandStarted, state: &PMState)
///         -> CommandResult<ProcessManagerHandleResponse> {
///         // ...
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn process_manager(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ProcessManagerArgs);
    let input = parse_macro_input!(item as ItemImpl);

    if let Some(err) = reject_stacked_kinds("process_manager", &input.attrs) {
        return TokenStream::from(err);
    }

    let expanded = expand_process_manager(args, input);
    TokenStream::from(expanded)
}

struct ProcessManagerArgs {
    name: String,
    pm_domain: String,
    state: Ident,
    sources: Vec<String>,
    #[allow(dead_code)] // consumed once the process_manager macro is R1-ified in R12
    targets: Vec<String>,
}

impl syn::parse::Parse for ProcessManagerArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut pm_domain = None;
        let mut state = None;
        let mut sources = None;
        let mut targets = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match ident.to_string().as_str() {
                "name" => {
                    let value: syn::LitStr = input.parse()?;
                    name = Some(value.value());
                }
                "pm_domain" => {
                    let value: syn::LitStr = input.parse()?;
                    pm_domain = Some(value.value());
                }
                "state" => {
                    let value: Ident = input.parse()?;
                    state = Some(value);
                }
                "sources" => {
                    sources = Some(parse_str_list(input)?);
                }
                "targets" => {
                    targets = Some(parse_str_list(input)?);
                }
                _ => return Err(syn::Error::new(ident.span(), "unknown attribute")),
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(ProcessManagerArgs {
            name: name.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "name is required")
            })?,
            pm_domain: pm_domain.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "pm_domain is required")
            })?,
            state: state.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "state is required")
            })?,
            sources: sources.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "sources is required")
            })?,
            targets: targets.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "targets is required")
            })?,
        })
    }
}

/// Parse a bracketed comma-separated list of string literals: `["a", "b", ...]`.
fn parse_str_list(input: syn::parse::ParseStream) -> syn::Result<Vec<String>> {
    let content;
    syn::bracketed!(content in input);
    let mut items = Vec::new();
    while !content.is_empty() {
        let lit: syn::LitStr = content.parse()?;
        items.push(lit.value());
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(items)
}

fn expand_process_manager(args: ProcessManagerArgs, mut input: ItemImpl) -> TokenStream2 {
    let name = &args.name;
    let pm_domain = &args.pm_domain;
    let state_ty = &args.state;
    let sources = &args.sources;
    let targets = &args.targets;
    let self_ty_for_applies = input.self_ty.clone();

    let meta = collect_method_metadata(&input);
    strip_method_markers(&mut input);

    let handled_exprs = meta
        .handled
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let applies_exprs = meta
        .applies
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let rejected_exprs = meta.rejected.iter().map(|(d, c)| {
        quote! { (#d.to_string(), #c.to_string()) }
    });
    let state_factory_expr = match &meta.state_factory {
        Some(name) => {
            let s = name.to_string();
            quote! { ::std::option::Option::Some(#s.to_string()) }
        }
        None => quote! { ::std::option::Option::None },
    };

    let dispatch_arms: Vec<TokenStream2> = meta
        .handled_with_methods
        .iter()
        .map(|(method_ident, evt_ty)| {
            quote! {
                if event_any.type_url == ::angzarr_client::full_type_url::<#evt_ty>() {
                    let evt = <#evt_ty as ::prost::Message>::decode(event_any.value.as_slice())
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(
                            ::std::format!("decode error for {}: {}", event_any.type_url, e)
                        ))?;
                    let response = self.#method_ident(evt, &state)
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(e.reason))?;
                    return ::std::result::Result::Ok(
                        ::angzarr_client::router::HandlerResponse::ProcessManager(response),
                    );
                }
            }
        })
        .collect();

    let apply_arms: Vec<TokenStream2> = meta
        .applies_with_methods
        .iter()
        .map(|(method_ident, evt_ty)| {
            quote! {
                if *evt_type_url == ::angzarr_client::full_type_url::<#evt_ty>() {
                    let evt_val = <#evt_ty as ::prost::Message>::decode(evt_any.value.as_slice())
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(
                            ::std::format!("decode error for {}: {}", evt_type_url, e)
                        ))?;
                    <#self_ty_for_applies>::#method_ident(&mut state, evt_val);
                    continue;
                }
            }
        })
        .collect();

    let initial_state_expr = match &meta.state_factory {
        Some(method_ident) => {
            quote! { <#self_ty_for_applies>::#method_ident() }
        }
        None => {
            quote! { <#state_ty as ::std::default::Default>::default() }
        }
    };

    let sources_vec = sources.iter().map(|s| quote! { #s.to_string() });
    let targets_vec = targets.iter().map(|s| quote! { #s.to_string() });

    let self_ty = &input.self_ty;
    quote! {
        #input

        impl ::angzarr_client::router::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::router::Kind =
                ::angzarr_client::router::Kind::ProcessManager;
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                ::angzarr_client::router::HandlerConfig::ProcessManager {
                    name: #name.to_string(),
                    pm_domain: #pm_domain.to_string(),
                    sources: ::std::vec![#(#sources_vec),*],
                    targets: ::std::vec![#(#targets_vec),*],
                    handled: ::std::vec![#(#handled_exprs),*],
                    rejected: ::std::vec![#(#rejected_exprs),*],
                    applies: ::std::vec![#(#applies_exprs),*],
                    state_factory: #state_factory_expr,
                }
            }

            fn dispatch(
                &self,
                request: ::angzarr_client::router::HandlerRequest,
            ) -> ::std::result::Result<
                ::angzarr_client::router::HandlerResponse,
                ::angzarr_client::ClientError,
            > {
                let pm_req = match request {
                    ::angzarr_client::router::HandlerRequest::ProcessManager(r) => r,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::InvalidArgument(
                                "process_manager dispatch requires HandlerRequest::ProcessManager".into(),
                            ),
                        );
                    }
                };

                // Rebuild PM state from process_state events.
                let mut state: #state_ty = #initial_state_expr;
                let process_state = pm_req.process_state.clone().unwrap_or_default();
                for page in &process_state.pages {
                    let evt_any = match &page.payload {
                        ::std::option::Option::Some(
                            ::angzarr_client::proto::event_page::Payload::Event(e),
                        ) => e,
                        _ => continue,
                    };
                    let evt_type_url = &evt_any.type_url;
                    #(#apply_arms)*
                }

                // Extract triggering event from trigger EventBook's last page.
                let trigger = pm_req.trigger.as_ref().ok_or_else(|| {
                    ::angzarr_client::ClientError::InvalidArgument("missing PM trigger".into())
                })?;
                let event_page = trigger.pages.last().ok_or_else(|| {
                    ::angzarr_client::ClientError::InvalidArgument("empty PM trigger".into())
                })?;
                let event_any = match &event_page.payload {
                    ::std::option::Option::Some(
                        ::angzarr_client::proto::event_page::Payload::Event(e),
                    ) => e,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::InvalidArgument(
                                "missing event payload on PM trigger".into(),
                            ),
                        );
                    }
                };

                #(#dispatch_arms)*

                ::std::result::Result::Ok(
                    ::angzarr_client::router::HandlerResponse::ProcessManager(
                        ::angzarr_client::proto::ProcessManagerHandleResponse::default(),
                    ),
                )
            }
        }
    }
}

/// Marks an impl block as a projector with event handlers.
///
/// # Attributes
/// - `name = "projector-name"` - The projector's name (required)
///
/// # Example
/// ```rust,ignore
/// #[projector(name = "output")]
/// impl OutputProjector {
///     #[projects(PlayerRegistered)]
///     fn project_registered(&self, event: PlayerRegistered) -> Projection {
///         // ...
///     }
///
///     #[projects(HandComplete)]
///     fn project_hand_complete(&self, event: HandComplete) -> Projection {
///         // ...
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn projector(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ProjectorArgs);
    let input = parse_macro_input!(item as ItemImpl);

    if let Some(err) = reject_stacked_kinds("projector", &input.attrs) {
        return TokenStream::from(err);
    }

    let expanded = expand_projector(args, input);
    TokenStream::from(expanded)
}

struct ProjectorArgs {
    name: String,
    #[allow(dead_code)] // consumed once the projector macro is R1-ified in R13
    domains: Vec<String>,
}

impl syn::parse::Parse for ProjectorArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut domains = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match ident.to_string().as_str() {
                "name" => {
                    let value: syn::LitStr = input.parse()?;
                    name = Some(value.value());
                }
                "domains" => {
                    domains = Some(parse_str_list(input)?);
                }
                _ => return Err(syn::Error::new(ident.span(), "unknown attribute")),
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(ProjectorArgs {
            name: name.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "name is required")
            })?,
            domains: domains.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "domains is required")
            })?,
        })
    }
}

fn expand_projector(args: ProjectorArgs, mut input: ItemImpl) -> TokenStream2 {
    let name = &args.name;
    let domains = &args.domains;

    let meta = collect_method_metadata(&input);
    strip_method_markers(&mut input);

    let handled_exprs = meta
        .handled
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let domains_vec = domains.iter().map(|d| quote! { #d.to_string() });

    let dispatch_arms: Vec<TokenStream2> = meta
        .handled_with_methods
        .iter()
        .map(|(method_ident, evt_ty)| {
            quote! {
                if event_any.type_url == ::angzarr_client::full_type_url::<#evt_ty>() {
                    let evt = <#evt_ty as ::prost::Message>::decode(event_any.value.as_slice())
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(
                            ::std::format!("decode error for {}: {}", event_any.type_url, e)
                        ))?;
                    self.#method_ident(evt)
                        .map_err(|e| ::angzarr_client::ClientError::InvalidArgument(e.reason))?;
                    continue;
                }
            }
        })
        .collect();

    let self_ty = &input.self_ty;
    quote! {
        #input

        impl ::angzarr_client::router::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::router::Kind =
                ::angzarr_client::router::Kind::Projector;
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                ::angzarr_client::router::HandlerConfig::Projector {
                    name: #name.to_string(),
                    domains: ::std::vec![#(#domains_vec),*],
                    handled: ::std::vec![#(#handled_exprs),*],
                }
            }

            fn dispatch(
                &self,
                request: ::angzarr_client::router::HandlerRequest,
            ) -> ::std::result::Result<
                ::angzarr_client::router::HandlerResponse,
                ::angzarr_client::ClientError,
            > {
                let book = match request {
                    ::angzarr_client::router::HandlerRequest::Projector(b) => b,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::InvalidArgument(
                                "projector dispatch requires HandlerRequest::Projector".into(),
                            ),
                        );
                    }
                };

                // R13: one instance, many events. Iterate each page and route
                // to the matching `#[handles]` arm. Side effects accumulate on
                // &self; no return value to merge.
                for page in &book.pages {
                    let event_any = match &page.payload {
                        ::std::option::Option::Some(
                            ::angzarr_client::proto::event_page::Payload::Event(e),
                        ) => e,
                        _ => continue,
                    };
                    #(#dispatch_arms)*
                    // Unmatched event type → skip silently.
                }

                // Return a skeleton Projection (no merged payload — side effects
                // are the projector's interface, not the return value).
                ::std::result::Result::Ok(
                    ::angzarr_client::router::HandlerResponse::Projector(
                        ::angzarr_client::proto::Projection {
                            cover: book.cover.clone(),
                            projector: #name.to_string(),
                            sequence: book.next_sequence,
                            projection: ::std::option::Option::None,
                        },
                    ),
                )
            }
        }
    }
}

// Helper functions

fn get_attr_ident(attr: &Attribute) -> syn::Result<Ident> {
    let meta = attr.meta.clone();
    match meta {
        Meta::List(list) => {
            let ident: Ident = syn::parse2(list.tokens)?;
            Ok(ident)
        }
        _ => Err(syn::Error::new_spanned(attr, "expected #[attr(Type)]")),
    }
}

fn get_rejected_args(attr: &Attribute) -> syn::Result<(String, String)> {
    let meta = attr.meta.clone();
    match meta {
        Meta::List(list) => {
            let args: RejectedArgs = syn::parse2(list.tokens)?;
            Ok((args.domain, args.command))
        }
        _ => Err(syn::Error::new_spanned(
            attr,
            "expected #[rejected(domain = \"...\", command = \"...\")]",
        )),
    }
}

struct RejectedArgs {
    domain: String,
    command: String,
}

impl syn::parse::Parse for RejectedArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut domain = None;
        let mut command = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: syn::LitStr = input.parse()?;

            match ident.to_string().as_str() {
                "domain" => domain = Some(value.value()),
                "command" => command = Some(value.value()),
                _ => return Err(syn::Error::new(ident.span(), "unknown attribute")),
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(RejectedArgs {
            domain: domain.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "domain is required")
            })?,
            command: command.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "command is required")
            })?,
        })
    }
}
