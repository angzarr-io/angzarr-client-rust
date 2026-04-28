//! Procedural macros for angzarr OO-style component definitions.
//!
//! # Command-handler Example
//!
//! ```rust,ignore
//! use angzarr_macros::{command_handler, handles, applies, rejected};
//!
//! #[command_handler(domain = "player", state = PlayerState)]
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
const KIND_ATTRS: &[&str] = &[
    "command_handler",
    "saga",
    "process_manager",
    "projector",
    "upcaster",
];

/// Take a parsed Option<String> and reject both absence and emptiness, mirroring
/// Python's `_require_non_empty_str` (`router/validation.py:43-45`). The
/// "is required" / "must be a non-empty string" wording matches Python's
/// `BuildError` messages.
fn require_non_empty_str(opt: Option<String>, field: &str) -> syn::Result<String> {
    match opt {
        None => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{} is required", field),
        )),
        Some(s) if s.is_empty() => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{} must be a non-empty string", field),
        )),
        Some(s) => Ok(s),
    }
}

/// Take a parsed Option<Vec<String>> and reject absence, emptiness, or any
/// empty element, mirroring Python's `_require_non_empty_list`
/// (`router/validation.py:48-50`).
fn require_non_empty_str_list(
    opt: Option<Vec<String>>,
    field: &str,
) -> syn::Result<Vec<String>> {
    match opt {
        None => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{} is required", field),
        )),
        Some(v) if v.is_empty() => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{} must be a non-empty list", field),
        )),
        Some(v) if v.iter().any(|s| s.is_empty()) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("{} must not contain empty strings", field),
        )),
        Some(v) => Ok(v),
    }
}

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
                    "#[{this_kind}] cannot coexist with #[{kind}] on the same impl; exactly one of #[command_handler] / #[saga] / #[process_manager] / #[projector] / #[upcaster] is allowed"
                );
                return Some(quote! { ::std::compile_error!(#msg); });
            }
        }
    }
    None
}

/// Marks an impl block as a command-handler aggregate. Cross-language
/// canonical name (matches Python's `@command_handler`).
///
/// # Attributes
/// - `domain = "name"` - The aggregate's domain name (required)
/// - `state = StateType` - The state type (required)
///
/// # Example
/// ```rust,ignore
/// #[command_handler(domain = "player", state = PlayerState)]
/// impl PlayerAggregate {
///     #[handles(RegisterPlayer)]
///     fn register(&self, cmd: RegisterPlayer, state: &PlayerState, seq: u32)
///         -> CommandResult<EventBook> {
///         // ...
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn command_handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as AggregateArgs);
    let input = parse_macro_input!(item as ItemImpl);

    if let Some(err) = reject_stacked_kinds("command_handler", &input.attrs) {
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
            domain: require_non_empty_str(domain, "domain")?,
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
                        .map_err(|e| ::angzarr_client::ClientError::invalid_argument(
                            ::angzarr_client::error_codes::codes::ANY_DECODE_FAILED,
                            ::angzarr_client::error_codes::messages::ANY_DECODE_FAILED,
                            [
                                (::angzarr_client::error_codes::keys::TYPE_URL, type_url.clone()),
                                (::angzarr_client::error_codes::keys::CAUSE, e.to_string()),
                            ],
                        ))?;
                    let events = self.#method_ident(cmd_val, &state, seq)
                        .map_err(::angzarr_client::ClientError::Rejected)?;
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
                        .map_err(|e| ::angzarr_client::ClientError::invalid_argument(
                            ::angzarr_client::error_codes::codes::ANY_DECODE_FAILED,
                            ::angzarr_client::error_codes::messages::ANY_DECODE_FAILED,
                            [
                                (::angzarr_client::error_codes::keys::TYPE_URL, evt_type_url.clone()),
                                (::angzarr_client::error_codes::keys::CAUSE, e.to_string()),
                            ],
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
                        .map_err(::angzarr_client::ClientError::Rejected)?;
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
                            ::angzarr_client::ClientError::invalid_argument(
                                ::angzarr_client::error_codes::codes::HANDLER_WRONG_REQUEST_KIND,
                                ::angzarr_client::error_codes::messages::HANDLER_WRONG_REQUEST_KIND,
                                [(
                                    ::angzarr_client::error_codes::keys::EXPECTED_KIND,
                                    "CommandHandler",
                                )],
                            ),
                        );
                    }
                };

                let cmd_book = ctx_cmd.command.as_ref().ok_or_else(|| {
                    ::angzarr_client::ClientError::invalid_argument(
                        ::angzarr_client::error_codes::codes::MISSING_COMMAND_BOOK,
                        ::angzarr_client::error_codes::messages::MISSING_COMMAND_BOOK,
                        ::std::iter::empty::<(&str, ::std::string::String)>(),
                    )
                })?;
                let cmd_page = cmd_book.pages.first().ok_or_else(|| {
                    ::angzarr_client::ClientError::invalid_argument(
                        ::angzarr_client::error_codes::codes::MISSING_COMMAND_PAGE,
                        ::angzarr_client::error_codes::messages::MISSING_COMMAND_PAGE,
                        ::std::iter::empty::<(&str, ::std::string::String)>(),
                    )
                })?;
                let payload = match &cmd_page.payload {
                    ::std::option::Option::Some(
                        ::angzarr_client::proto::command_page::Payload::Command(c),
                    ) => c,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::invalid_argument(
                                ::angzarr_client::error_codes::codes::MISSING_COMMAND_PAYLOAD,
                                ::angzarr_client::error_codes::messages::MISSING_COMMAND_PAYLOAD,
                                ::std::iter::empty::<(&str, ::std::string::String)>(),
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
                        .map_err(|e| ::angzarr_client::ClientError::invalid_argument(
                            ::angzarr_client::error_codes::codes::NOTIFICATION_DECODE_FAILED,
                            ::angzarr_client::error_codes::messages::NOTIFICATION_DECODE_FAILED,
                            [(::angzarr_client::error_codes::keys::CAUSE, e.to_string())],
                        ))?;

                    let rejection = match notification.payload.as_ref() {
                        ::std::option::Option::Some(p) => {
                            <::angzarr_client::proto::RejectionNotification as ::prost::Message>::decode(
                                p.value.as_slice(),
                            )
                            .map_err(|e| ::angzarr_client::ClientError::invalid_argument(
                                ::angzarr_client::error_codes::codes::REJECTION_NOTIFICATION_DECODE_FAILED,
                                ::angzarr_client::error_codes::messages::REJECTION_NOTIFICATION_DECODE_FAILED,
                                [(::angzarr_client::error_codes::keys::CAUSE, e.to_string())],
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
                    ::angzarr_client::ClientError::invalid_argument(
                        ::angzarr_client::error_codes::codes::NO_HANDLER_REGISTERED,
                        ::angzarr_client::error_codes::messages::NO_HANDLER_REGISTERED,
                        [(::angzarr_client::error_codes::keys::TYPE_URL, type_url.clone())],
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
    /// `(method name, from type, to type)` triples for upcaster dispatch arms.
    upcasts_with_methods: Vec<(Ident, Ident, Ident)>,
}

fn collect_method_metadata(input: &ItemImpl) -> MethodMetadata {
    let mut handled = Vec::new();
    let mut handled_with_methods = Vec::new();
    let mut rejected = Vec::new();
    let mut rejected_with_methods = Vec::new();
    let mut applies = Vec::new();
    let mut applies_with_methods = Vec::new();
    let mut state_factory = None;
    let mut upcasts_with_methods = Vec::new();

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
            } else if attr.path().is_ident("upcasts") {
                if let Ok((from, to)) = get_upcasts_args(attr) {
                    upcasts_with_methods.push((method.sig.ident.clone(), from, to));
                }
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
        upcasts_with_methods,
    }
}

fn get_upcasts_args(attr: &Attribute) -> syn::Result<(Ident, Ident)> {
    let meta = attr.meta.clone();
    match meta {
        Meta::List(list) => {
            let args: UpcastsArgsParse = syn::parse2(list.tokens)?;
            Ok((args.from, args.to))
        }
        _ => Err(syn::Error::new_spanned(
            attr,
            "expected #[upcasts(from = FromType, to = ToType)]",
        )),
    }
}

struct UpcastsArgsParse {
    from: Ident,
    to: Ident,
}

impl syn::parse::Parse for UpcastsArgsParse {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut from = None;
        let mut to = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: Ident = input.parse()?;

            match ident.to_string().as_str() {
                "from" => from = Some(value),
                "to" => to = Some(value),
                _ => return Err(syn::Error::new(ident.span(), "unknown attribute")),
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(UpcastsArgsParse {
            from: from.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "from is required")
            })?,
            to: to
                .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "to is required"))?,
        })
    }
}

/// Strip `#[handles]`, `#[applies]`, `#[rejected]`, `#[state_factory]`,
/// `#[upcasts]` from the methods of an impl block so rustc doesn't see
/// them as unknown attrs.
fn strip_method_markers(input: &mut ItemImpl) {
    for item in &mut input.items {
        if let ImplItem::Fn(method) = item {
            method.attrs.retain(|attr| {
                !attr.path().is_ident("handles")
                    && !attr.path().is_ident("rejected")
                    && !attr.path().is_ident("applies")
                    && !attr.path().is_ident("state_factory")
                    && !attr.path().is_ident("upcasts")
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
    // The actual work is done by the #[command_handler] macro
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
    // The actual work is done by the #[command_handler] or #[process_manager] macro
    // This is just a marker attribute
    item
}

/// Marks a method as an event applier for state reconstruction.
///
/// The method must be a static function with signature:
/// `fn(state: &mut State, event: EventType)`
///
/// The #[command_handler] macro collects these and generates:
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
    // The actual work is done by the #[command_handler] macro
    // This is just a marker attribute
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
            name: require_non_empty_str(name, "name")?,
            source: require_non_empty_str(source, "source")?,
            target: require_non_empty_str(target, "target")?,
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
                        .map_err(|e| ::angzarr_client::ClientError::invalid_argument(
                            ::angzarr_client::error_codes::codes::ANY_DECODE_FAILED,
                            ::angzarr_client::error_codes::messages::ANY_DECODE_FAILED,
                            [
                                (::angzarr_client::error_codes::keys::TYPE_URL, event_any.type_url.clone()),
                                (::angzarr_client::error_codes::keys::CAUSE, e.to_string()),
                            ],
                        ))?;
                    let response = self.#method_ident(evt)
                        .map_err(::angzarr_client::ClientError::Rejected)?;
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
                            ::angzarr_client::ClientError::invalid_argument(
                                ::angzarr_client::error_codes::codes::HANDLER_WRONG_REQUEST_KIND,
                                ::angzarr_client::error_codes::messages::HANDLER_WRONG_REQUEST_KIND,
                                [(
                                    ::angzarr_client::error_codes::keys::EXPECTED_KIND,
                                    "Saga",
                                )],
                            ),
                        );
                    }
                };

                let source_book = saga_req.source.as_ref().ok_or_else(|| {
                    ::angzarr_client::ClientError::invalid_argument(
                        ::angzarr_client::error_codes::codes::MISSING_SAGA_SOURCE,
                        ::angzarr_client::error_codes::messages::MISSING_SAGA_SOURCE,
                        ::std::iter::empty::<(&str, ::std::string::String)>(),
                    )
                })?;
                let event_page = source_book.pages.last().ok_or_else(|| {
                    ::angzarr_client::ClientError::invalid_argument(
                        ::angzarr_client::error_codes::codes::EMPTY_SAGA_SOURCE,
                        ::angzarr_client::error_codes::messages::EMPTY_SAGA_SOURCE,
                        ::std::iter::empty::<(&str, ::std::string::String)>(),
                    )
                })?;
                let event_any = match &event_page.payload {
                    ::std::option::Option::Some(
                        ::angzarr_client::proto::event_page::Payload::Event(e),
                    ) => e,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::invalid_argument(
                                ::angzarr_client::error_codes::codes::MISSING_SAGA_EVENT_PAYLOAD,
                                ::angzarr_client::error_codes::messages::MISSING_SAGA_EVENT_PAYLOAD,
                                ::std::iter::empty::<(&str, ::std::string::String)>(),
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
            name: require_non_empty_str(name, "name")?,
            pm_domain: require_non_empty_str(pm_domain, "pm_domain")?,
            state: state.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "state is required")
            })?,
            sources: require_non_empty_str_list(sources, "sources")?,
            targets: require_non_empty_str_list(targets, "targets")?,
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
                        .map_err(|e| ::angzarr_client::ClientError::invalid_argument(
                            ::angzarr_client::error_codes::codes::ANY_DECODE_FAILED,
                            ::angzarr_client::error_codes::messages::ANY_DECODE_FAILED,
                            [
                                (::angzarr_client::error_codes::keys::TYPE_URL, event_any.type_url.clone()),
                                (::angzarr_client::error_codes::keys::CAUSE, e.to_string()),
                            ],
                        ))?;
                    let response = self.#method_ident(evt, &state)
                        .map_err(::angzarr_client::ClientError::Rejected)?;
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
                        .map_err(|e| ::angzarr_client::ClientError::invalid_argument(
                            ::angzarr_client::error_codes::codes::ANY_DECODE_FAILED,
                            ::angzarr_client::error_codes::messages::ANY_DECODE_FAILED,
                            [
                                (::angzarr_client::error_codes::keys::TYPE_URL, evt_type_url.clone()),
                                (::angzarr_client::error_codes::keys::CAUSE, e.to_string()),
                            ],
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
                            ::angzarr_client::ClientError::invalid_argument(
                                ::angzarr_client::error_codes::codes::HANDLER_WRONG_REQUEST_KIND,
                                ::angzarr_client::error_codes::messages::HANDLER_WRONG_REQUEST_KIND,
                                [(
                                    ::angzarr_client::error_codes::keys::EXPECTED_KIND,
                                    "ProcessManager",
                                )],
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
                    ::angzarr_client::ClientError::invalid_argument(
                        ::angzarr_client::error_codes::codes::MISSING_PM_TRIGGER,
                        ::angzarr_client::error_codes::messages::MISSING_PM_TRIGGER,
                        ::std::iter::empty::<(&str, ::std::string::String)>(),
                    )
                })?;
                let event_page = trigger.pages.last().ok_or_else(|| {
                    ::angzarr_client::ClientError::invalid_argument(
                        ::angzarr_client::error_codes::codes::EMPTY_PM_TRIGGER,
                        ::angzarr_client::error_codes::messages::EMPTY_PM_TRIGGER,
                        ::std::iter::empty::<(&str, ::std::string::String)>(),
                    )
                })?;
                let event_any = match &event_page.payload {
                    ::std::option::Option::Some(
                        ::angzarr_client::proto::event_page::Payload::Event(e),
                    ) => e,
                    _ => {
                        return ::std::result::Result::Err(
                            ::angzarr_client::ClientError::invalid_argument(
                                ::angzarr_client::error_codes::codes::MISSING_PM_EVENT_PAYLOAD,
                                ::angzarr_client::error_codes::messages::MISSING_PM_EVENT_PAYLOAD,
                                ::std::iter::empty::<(&str, ::std::string::String)>(),
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
            name: require_non_empty_str(name, "name")?,
            domains: require_non_empty_str_list(domains, "domains")?,
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
                        .map_err(|e| ::angzarr_client::ClientError::invalid_argument(
                            ::angzarr_client::error_codes::codes::ANY_DECODE_FAILED,
                            ::angzarr_client::error_codes::messages::ANY_DECODE_FAILED,
                            [
                                (::angzarr_client::error_codes::keys::TYPE_URL, event_any.type_url.clone()),
                                (::angzarr_client::error_codes::keys::CAUSE, e.to_string()),
                            ],
                        ))?;
                    self.#method_ident(evt)
                        .map_err(::angzarr_client::ClientError::Rejected)?;
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
                            ::angzarr_client::ClientError::invalid_argument(
                                ::angzarr_client::error_codes::codes::HANDLER_WRONG_REQUEST_KIND,
                                ::angzarr_client::error_codes::messages::HANDLER_WRONG_REQUEST_KIND,
                                [(
                                    ::angzarr_client::error_codes::keys::EXPECTED_KIND,
                                    "Projector",
                                )],
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
            domain: require_non_empty_str(domain, "domain")?,
            command: require_non_empty_str(command, "command")?,
        })
    }
}

/// Marks a method as the state factory for its aggregate / process manager.
///
/// The method must be a static function returning the state type. When a
/// handler's `#[command_handler]` or `#[process_manager]` macro sees a
/// method annotated with `#[state_factory]`, it calls that method to
/// construct the initial state instead of `Default::default()`.
///
/// Exposed as a standalone proc macro so cross-language docs / examples can
/// reference it via the same name (`@state_factory` in Python,
/// `#[state_factory]` in Rust). The actual work is done by the parent
/// kind macro.
#[proc_macro_attribute]
pub fn state_factory(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Marker attribute — parent kind macros strip and consume it.
    item
}

/// Marks a method as an event version transformation.
///
/// # Attributes
/// - `from = OldType` — the proto message type this method accepts
/// - `to = NewType` — the proto message type this method produces
///
/// # Example
/// ```rust,ignore
/// #[upcasts(from = PlayerRegisteredV1, to = PlayerRegisteredV2)]
/// fn upgrade(old: PlayerRegisteredV1) -> PlayerRegisteredV2 {
///     PlayerRegisteredV2 { /* ... */ }
/// }
/// ```
///
/// Consumed by the parent `#[upcaster]` macro at expansion time. Validates
/// attribute shape (both `from` and `to` are required) but otherwise
/// passes the method through unchanged.
#[proc_macro_attribute]
pub fn upcasts(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as UpcastsArgs);
    let _ = args; // consumed by parent #[upcaster]; attrs validated here
    item
}

struct UpcastsArgs {
    #[allow(dead_code)]
    from: Ident,
    #[allow(dead_code)]
    to: Ident,
}

impl syn::parse::Parse for UpcastsArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut from = None;
        let mut to = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match ident.to_string().as_str() {
                "from" => from = Some(input.parse()?),
                "to" => to = Some(input.parse()?),
                _ => {
                    return Err(syn::Error::new(
                        ident.span(),
                        "unknown attribute: expected `from` or `to`",
                    ))
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(UpcastsArgs {
            from: from.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "from is required")
            })?,
            to: to
                .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "to is required"))?,
        })
    }
}

/// Marks a class as an upcaster — transforms events in a given `domain`
/// from one version to another.
///
/// # Attributes
/// - `name = "..."` — identifier for this upcaster (used in logging / registry)
/// - `domain = "..."` — the aggregate domain whose events this upcaster covers
///
/// # Example
/// ```rust,ignore
/// #[upcaster(name = "player-v1-to-v2", domain = "player")]
/// impl PlayerUpcaster {
///     #[upcasts(from = PlayerRegisteredV1, to = PlayerRegisteredV2)]
///     fn upgrade(old: PlayerRegisteredV1) -> PlayerRegisteredV2 { /* ... */ }
/// }
///
/// let router = Router::new("upcaster-player")
///     .with_handler(|| PlayerUpcaster)
///     .build()?
///     .into_upcaster()?; // or match Built::Upcaster(r)
/// ```
///
/// The macro emits `impl HandlerKind` + `impl Handler` on the annotated
/// type so the unified `Router` builder (from the `angzarr-client` crate)
/// accepts it as a homogeneous factory. Dispatch matches each incoming
/// event's type URL against the registered `#[upcasts(from = …, to = …)]`
/// pairs and invokes the first
/// matching method; events without a matching transform pass through.
#[proc_macro_attribute]
pub fn upcaster(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as UpcasterArgs);
    let input = parse_macro_input!(item as ItemImpl);

    if let Some(err) = reject_stacked_kinds("upcaster", &input.attrs) {
        return TokenStream::from(err);
    }

    let expanded = expand_upcaster(args, input);
    TokenStream::from(expanded)
}

fn expand_upcaster(args: UpcasterArgs, mut input: ItemImpl) -> TokenStream2 {
    let name = &args.name;
    let domain = &args.domain;
    let self_ty = input.self_ty.clone();

    let meta = collect_method_metadata(&input);
    strip_method_markers(&mut input);

    // (from_url, to_url) pairs for HandlerConfig::Upcaster.upcasts
    let upcast_pairs = meta.upcasts_with_methods.iter().map(|(_, from, to)| {
        quote! {
            (
                ::angzarr_client::full_type_url::<#from>(),
                ::angzarr_client::full_type_url::<#to>(),
            )
        }
    });

    // Dispatch arms: match each method's `from` type URL against the incoming
    // event's type URL; the first matching method transforms it.
    let dispatch_arms = meta
        .upcasts_with_methods
        .iter()
        .map(|(method_ident, from, to)| {
            quote! {
                if event_any.type_url == ::angzarr_client::full_type_url::<#from>() {
                    let old = <#from as ::prost::Message>::decode(event_any.value.as_slice())
                        .map_err(|e| ::angzarr_client::ClientError::invalid_argument(
                            ::angzarr_client::error_codes::codes::ANY_DECODE_FAILED,
                            ::angzarr_client::error_codes::messages::ANY_DECODE_FAILED,
                            [
                                (::angzarr_client::error_codes::keys::TYPE_URL, event_any.type_url.clone()),
                                (::angzarr_client::error_codes::keys::CAUSE, e.to_string()),
                            ],
                        ))?;
                    let new: #to = <#self_ty>::#method_ident(old);
                    let new_any = ::prost_types::Any {
                        type_url: ::angzarr_client::full_type_url::<#to>(),
                        value: ::prost::Message::encode_to_vec(&new),
                    };
                    out_pages.push(::angzarr_client::proto::EventPage {
                        header: page.header.clone(),
                        created_at: page.created_at,
                        payload: Some(::angzarr_client::proto::event_page::Payload::Event(new_any)),
                        no_commit: page.no_commit,
                        cascade_id: page.cascade_id.clone(),
                    });
                    continue;
                }
            }
        });

    quote! {
        #input

        impl ::angzarr_client::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::Kind = ::angzarr_client::Kind::Upcaster;
        }

        impl ::angzarr_client::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::HandlerConfig {
                ::angzarr_client::HandlerConfig::Upcaster {
                    name: #name.to_string(),
                    domain: #domain.to_string(),
                    upcasts: vec![#( #upcast_pairs ),*],
                }
            }

            fn dispatch(
                &self,
                request: ::angzarr_client::HandlerRequest,
            ) -> ::core::result::Result<
                ::angzarr_client::HandlerResponse,
                ::angzarr_client::ClientError,
            > {
                let req = match request {
                    ::angzarr_client::HandlerRequest::Upcaster(r) => r,
                    _ => {
                        return Err(::angzarr_client::ClientError::invalid_argument(
                            ::angzarr_client::error_codes::codes::HANDLER_WRONG_REQUEST_KIND,
                            ::angzarr_client::error_codes::messages::HANDLER_WRONG_REQUEST_KIND,
                            [(
                                ::angzarr_client::error_codes::keys::EXPECTED_KIND,
                                "Upcaster",
                            )],
                        ));
                    }
                };

                let mut out_pages: Vec<::angzarr_client::proto::EventPage> =
                    Vec::with_capacity(req.events.len());

                'page: for page in req.events.iter() {
                    // Only event payloads are candidates for upcasting; external
                    // payloads and headerless pages pass through untouched.
                    let Some(::angzarr_client::proto::event_page::Payload::Event(ref event_any)) =
                        page.payload
                    else {
                        out_pages.push(page.clone());
                        continue 'page;
                    };

                    #( #dispatch_arms )*

                    // No matching upcast — pass the page through unchanged.
                    out_pages.push(page.clone());
                }

                Ok(::angzarr_client::HandlerResponse::Upcaster(
                    ::angzarr_client::proto::UpcastResponse { events: out_pages },
                ))
            }
        }
    }
}

struct UpcasterArgs {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    domain: String,
}

impl syn::parse::Parse for UpcasterArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut domain = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match ident.to_string().as_str() {
                "name" => {
                    let value: syn::LitStr = input.parse()?;
                    name = Some(value.value());
                }
                "domain" => {
                    let value: syn::LitStr = input.parse()?;
                    domain = Some(value.value());
                }
                _ => {
                    return Err(syn::Error::new(
                        ident.span(),
                        "unknown attribute: expected `name` or `domain`",
                    ))
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(UpcasterArgs {
            name: require_non_empty_str(name, "name")?,
            domain: require_non_empty_str(domain, "domain")?,
        })
    }
}
