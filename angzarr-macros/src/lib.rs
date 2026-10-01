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
//!     fn register(&self, cmd: RegisterPlayer, state: &PlayerState, seq: u32)
//!         -> CommandResult<EventBook> {
//!         // ...
//!     }
//!
//!     #[rejected(domain = "payment", command = ProcessPayment)]
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
fn require_non_empty_str_list(opt: Option<Vec<String>>, field: &str) -> syn::Result<Vec<String>> {
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
    /// Audit #45: opt-in for the `Replay` RPC. When `true`, the framework
    /// auto-implements replay using existing `#[applies]` methods. The
    /// gRPC adapter gates on this metadata — `false` → UNIMPLEMENTED.
    supports_replay: bool,
}

impl syn::parse::Parse for AggregateArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut domain = None;
        let mut state = None;
        let mut supports_replay = false;

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
                "supports_replay" => {
                    let value: syn::LitBool = input.parse()?;
                    supports_replay = value.value;
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
            supports_replay,
        })
    }
}

fn expand_aggregate(args: AggregateArgs, mut input: ItemImpl) -> TokenStream2 {
    let domain = &args.domain;
    let state_ty = &args.state;
    let supports_replay = args.supports_replay;

    let meta = match collect_method_metadata(&input) {
        Ok(meta) => meta,
        Err(e) => return e.to_compile_error(),
    };
    strip_method_markers(&mut input);
    let self_ty = &input.self_ty;

    let handled_exprs = meta
        .handled
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let applies_exprs = meta
        .applies
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let compensates_exprs = meta
        .rejected_with_methods
        .iter()
        .map(|(_, d, c)| compensates_key(d, c));
    let handles_fact_exprs = meta
        .handles_fact
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let state_factory_expr = match &meta.state_factory {
        Some(name) => {
            let s = name.to_string();
            quote! { ::std::option::Option::Some(#s.to_string()) }
        }
        None => quote! { ::std::option::Option::None },
    };

    let rebuilder = rebuilder_expr(self_ty, state_ty, &meta);

    let command_regs = meta.handled_with_methods.iter().map(|(method, cmd_ty)| {
        quote! {
            let f = ::std::sync::Arc::clone(&factory);
            table = table.on_command(
                &<#cmd_ty as ::prost::Name>::full_name(),
                move |any, state, ctx| {
                    let cmd: #cmd_ty = __p::decode(any)?;
                    let handler = f();
                    handler
                        .#method(cmd, &*state, ctx.next_sequence)
                        .map(::std::option::Option::Some)
                        .map_err(__p::rejected)
                },
            );
        }
    });
    let rejection_regs = meta.rejected_with_methods.iter().map(|(method, d, c)| {
        let key = compensates_key(d, c);
        quote! {
            let f = ::std::sync::Arc::clone(&factory);
            table = table.on_rejected(&#key, move |notification, _rejection, state, _ctx| {
                let handler = f();
                handler.#method(notification, &*state).map_err(__p::rejected)
            });
        }
    });
    let fact_regs = meta
        .handles_fact_with_methods
        .iter()
        .map(|(method, fact_ty)| {
            quote! {
                let f = ::std::sync::Arc::clone(&factory);
                table = table.on_fact(&<#fact_ty as ::prost::Name>::full_name(), move |any, state| {
                    let fact: #fact_ty = __p::decode(any)?;
                    let handler = f();
                    let recorded = handler.#method(fact, state).map_err(__p::rejected)?;
                    ::std::result::Result::Ok(__p::fact_record(recorded))
                });
            }
        });
    let replay_expr = if supports_replay {
        quote! { let table = table.with_message_state(); }
    } else {
        quote! {}
    };

    let config_expr: TokenStream2 = quote! {
        ::angzarr_client::router::HandlerConfig::CommandHandler {
            domain: #domain.to_string(),
            handled: ::std::vec![#(#handled_exprs),*],
            compensates: ::std::vec![#(#compensates_exprs),*],
            applies: ::std::vec![#(#applies_exprs),*],
            state_factory: #state_factory_expr,
            handles_fact: ::std::vec![#(#handles_fact_exprs),*],
            supports_replay: #supports_replay,
        }
    };
    let name = quote!(#self_ty).to_string();

    quote! {
        #input

        impl ::angzarr_client::router::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::router::Kind =
                ::angzarr_client::router::Kind::CommandHandler;
            fn handler_config() -> ::angzarr_client::router::HandlerConfig {
                #config_expr
            }
            fn component(
                factory: ::angzarr_client::router::component::Factory<Self>,
            ) -> ::angzarr_client::router::component::Component {
                use ::angzarr_client::router::component as __p;
                #[allow(unused_mut)]
                let mut table = ::angzarr_client::__router::aggregate::AggregateDispatch::new(
                    #name, #domain, #rebuilder,
                );
                #(#command_regs)*
                #(#rejection_regs)*
                #(#fact_regs)*
                let _ = &factory;
                #replay_expr
                __p::Component::CommandHandler(::std::boxed::Box::new(table))
            }
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                <#self_ty as ::angzarr_client::router::HandlerKind>::handler_config()
            }
        }
    }
}

/// An angzarr-router `Rebuilder<S>` for the impl's state: the
/// `#[state_factory]` (or `Default`), every `#[applies]` method, and the
/// snapshot loader when `S` is a protobuf message.
fn rebuilder_expr(self_ty: &syn::Type, state_ty: &Ident, meta: &MethodMetadata) -> TokenStream2 {
    let initial = match &meta.state_factory {
        Some(method) => quote! { <#self_ty>::#method() },
        None => quote! { <#state_ty as ::std::default::Default>::default() },
    };
    let appliers = meta.applies_with_methods.iter().map(|(method, evt_ty)| {
        quote! {
            let rebuilder = rebuilder.apply(
                &<#evt_ty as ::prost::Name>::full_name(),
                |state: &mut #state_ty, any| {
                    let event: #evt_ty = ::angzarr_client::router::component::decode_applied(any)?;
                    <#self_ty>::#method(state, event);
                    ::std::result::Result::Ok(())
                },
            );
        }
    });
    quote! {{
        use ::angzarr_client::router::component::{IgnoresSnapshot as _, LoadsSnapshot as _};
        let rebuilder = ::angzarr_client::__router::rebuild::Rebuilder::<#state_ty>::new(|| #initial);
        #(#appliers)*
        ::angzarr_client::router::component::with_snapshot(
            rebuilder,
            (&::angzarr_client::router::component::SnapshotState::<#state_ty>::new())
                .snapshot_loader(),
        )
    }}
}

/// Which component a handler's optional context parameters come from.
#[derive(Clone, Copy, PartialEq)]
enum ContextKind {
    /// `dests` and the triggering `page` context are in scope.
    Saga,
    /// `dests` and `source_cover` are in scope.
    ProcessManager,
}

/// The optional context parameters a saga / process-manager handler may
/// declare after its required ones, by name: `destinations` (the declared
/// output domains), `source_cover` (the triggering book's cover) and, for a
/// saga, `source_seq` (the triggering event's sequence).
fn context_args(
    method: &syn::ImplItemFn,
    required: usize,
    kind: ContextKind,
) -> syn::Result<Vec<TokenStream2>> {
    let mut out = Vec::new();
    for arg in method.sig.inputs.iter().skip(1 + required) {
        let syn::FnArg::Typed(pat) = arg else {
            continue;
        };
        let syn::Pat::Ident(ident) = &*pat.pat else {
            return Err(syn::Error::new_spanned(
                &pat.pat,
                "context parameters are matched by name; use a plain identifier",
            ));
        };
        match (ident.ident.to_string().as_str(), kind) {
            ("destinations", _) => out.push(quote! { dests }),
            ("source_cover", ContextKind::Saga) => out.push(quote! { page.cover.cloned() }),
            ("source_cover", ContextKind::ProcessManager) => {
                out.push(quote! { source_cover.cloned() })
            }
            ("source_seq", ContextKind::Saga) => out.push(quote! { page.sequence }),
            (other, ContextKind::Saga) => {
                return Err(syn::Error::new_spanned(
                    &ident.ident,
                    format!(
                        "unsupported handler parameter `{other}`: optional parameters are \
                         `destinations`, `source_cover` and `source_seq`"
                    ),
                ))
            }
            (other, ContextKind::ProcessManager) => {
                return Err(syn::Error::new_spanned(
                    &ident.ident,
                    format!(
                        "unsupported handler parameter `{other}`: optional parameters are \
                         `destinations` and `source_cover`"
                    ),
                ))
            }
        }
    }
    Ok(out)
}

/// The method named `name` in the impl.
fn find_method<'a>(input: &'a ItemImpl, name: &Ident) -> &'a syn::ImplItemFn {
    input
        .items
        .iter()
        .find_map(|item| match item {
            ImplItem::Fn(m) if &m.sig.ident == name => Some(m),
            _ => None,
        })
        .expect("metadata names a method of this impl")
}

/// Metadata harvested from method-level attribute markers inside a kind-impl.
struct MethodMetadata {
    /// Types named in `#[handles(T)]` (kept for config emission).
    handled: Vec<syn::Path>,
    /// `(method name, command type)` pairs for dispatch-arm generation.
    handled_with_methods: Vec<(Ident, syn::Path)>,
    /// `(method name, domain qualifier, command type)` from `#[rejected]`.
    rejected_with_methods: Vec<(Ident, Option<String>, syn::Path)>,
    /// Types named in `#[applies(T)]` (kept for config emission).
    applies: Vec<syn::Path>,
    /// `(method name, event type)` pairs for state-rebuild-arm generation.
    applies_with_methods: Vec<(Ident, syn::Path)>,
    /// Name of the method annotated with `#[state_factory]`, if any.
    state_factory: Option<Ident>,
    /// `(method name, from type, to type)` triples for upcaster dispatch arms.
    upcasts_with_methods: Vec<(Ident, syn::Path, syn::Path)>,
    /// Types named in `#[handles_fact(T)]` for config emission.
    handles_fact: Vec<syn::Path>,
    /// `(method name, fact event type)` pairs for HandleFact
    /// dispatch-arm generation.
    handles_fact_with_methods: Vec<(Ident, syn::Path)>,
}

/// Method-level markers a kind macro consumes. A method carries at most one
/// of them: a handler, applier, compensator, state factory and upcast are
/// different roles with different signatures.
const METHOD_MARKERS: &[&str] = &[
    "handles",
    "handles_fact",
    "applies",
    "rejected",
    "state_factory",
    "upcasts",
];

fn marker_name(attr: &Attribute) -> Option<&'static str> {
    METHOD_MARKERS
        .iter()
        .copied()
        .find(|m| attr.path().is_ident(m))
}

/// Collect every method marker of a kind impl. A malformed marker, or a
/// method carrying two different markers, is a compile error (all such
/// errors are reported together) rather than a silently unrouted method.
fn collect_method_metadata(input: &ItemImpl) -> syn::Result<MethodMetadata> {
    let mut meta = MethodMetadata {
        handled: Vec::new(),
        handled_with_methods: Vec::new(),
        rejected_with_methods: Vec::new(),
        applies: Vec::new(),
        applies_with_methods: Vec::new(),
        state_factory: None,
        upcasts_with_methods: Vec::new(),
        handles_fact: Vec::new(),
        handles_fact_with_methods: Vec::new(),
    };
    let mut errors: Option<syn::Error> = None;
    let mut push_err = |e: syn::Error| match errors.as_mut() {
        Some(acc) => acc.combine(e),
        None => errors = Some(e),
    };

    for item in &input.items {
        let ImplItem::Fn(method) = item else { continue };
        let name = &method.sig.ident;
        let mut role: Option<&'static str> = None;
        for attr in &method.attrs {
            let Some(marker) = marker_name(attr) else {
                continue;
            };
            match role {
                Some(first) if first != marker => push_err(syn::Error::new_spanned(
                    attr,
                    format!(
                        "#[{marker}] conflicts with #[{first}] on `{name}`: a method takes exactly one role"
                    ),
                )),
                _ => role = Some(marker),
            }
            match marker {
                "handles" => match get_attr_path(attr) {
                    Ok(ty) => {
                        meta.handled.push(ty.clone());
                        meta.handled_with_methods.push((name.clone(), ty));
                    }
                    Err(e) => push_err(e),
                },
                "handles_fact" => match get_attr_path(attr) {
                    Ok(ty) => {
                        meta.handles_fact.push(ty.clone());
                        meta.handles_fact_with_methods.push((name.clone(), ty));
                    }
                    Err(e) => push_err(e),
                },
                "applies" => match get_attr_path(attr) {
                    Ok(ty) => {
                        meta.applies.push(ty.clone());
                        meta.applies_with_methods.push((name.clone(), ty));
                    }
                    Err(e) => push_err(e),
                },
                "rejected" => match get_rejected_args(attr) {
                    Ok((d, c)) => meta.rejected_with_methods.push((name.clone(), d, c)),
                    Err(e) => push_err(e),
                },
                "state_factory" => {
                    if !matches!(attr.meta, Meta::Path(_)) {
                        push_err(syn::Error::new_spanned(
                            attr,
                            "#[state_factory] takes no arguments",
                        ));
                    } else if let Some(prev) = &meta.state_factory {
                        push_err(syn::Error::new_spanned(
                            attr,
                            format!("#[state_factory] is already declared on `{prev}`"),
                        ));
                    } else {
                        meta.state_factory = Some(name.clone());
                    }
                }
                "upcasts" => match get_upcasts_args(attr) {
                    Ok((from, to)) => meta.upcasts_with_methods.push((name.clone(), from, to)),
                    Err(e) => push_err(e),
                },
                _ => {}
            }
        }
    }

    match errors {
        Some(e) => Err(e),
        None => Ok(meta),
    }
}

fn get_upcasts_args(attr: &Attribute) -> syn::Result<(syn::Path, syn::Path)> {
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
    from: syn::Path,
    to: syn::Path,
}

impl syn::parse::Parse for UpcastsArgsParse {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut from = None;
        let mut to = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: syn::Path = input.parse()?;

            match ident.to_string().as_str() {
                "from" => from = Some(value),
                "to" => to = Some(value),
                _ => {
                    return Err(syn::Error::new(
                        ident.span(),
                        "unknown #[upcasts] argument: expected `from` or `to`",
                    ))
                }
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
                    && !attr.path().is_ident("handles_fact")
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

/// Marks a method as a fact handler.
///
/// Triggered when the coordinator dispatches a fact (an external
/// reality, e.g. a payment confirmation) via the `HandleFact` RPC. The
/// method receives `(self, fact, state)` after state has been rebuilt
/// from prior events (and the facts and flags recorded before it in the
/// same request). It returns the fact to record, optionally followed by
/// events that flag it, as a `FactRecord`; returning the fact message
/// itself records it with no flags. Facts cannot be refused.
///
/// Each `#[handles_fact]` type is a `ComponentOptions.facts` entry. A fact
/// of any other type is refused with INVALID_ARGUMENT / NO_FACT_HANDLER and
/// nothing is recorded.
///
/// # Example
/// ```rust,ignore
/// #[handles_fact(ShipmentDispatched)]
/// fn on_dispatched(&self, fact: ShipmentDispatched, state: &OrderState)
///     -> CommandResult<FactRecord> {
///     let record = FactRecord::new(&fact);
///     Ok(if state.awaiting_shipment {
///         record
///     } else {
///         record.flag(&ShipmentDiscrepancy { order_id: fact.order_id })
///     })
/// }
/// ```
#[proc_macro_attribute]
pub fn handles_fact(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Marker attribute; the #[command_handler] macro reads it during
    // expansion to build the fact-dispatch table.
    item
}

/// Marks a method as a rejection (compensation) handler on a command
/// handler or process manager — a `compensates` entry.
///
/// # Attributes
/// - `command = Type` - The rejected command's type (required); matched by
///   its fully-qualified name
/// - `domain = "name"` - Only when the command was sent to this domain
///   (optional; without it any domain matches)
///
/// A `#[saga]` never receives rejections; `#[rejected]` there is a compile
/// error.
///
/// # Example
/// ```rust,ignore
/// #[rejected(domain = "payment", command = ProcessPayment)]
/// fn handle_payment_rejected(&self, notification: &Notification, state: &PlayerState)
///     -> CommandResult<BusinessResponse> {
///     // ...
/// }
/// ```
#[proc_macro_attribute]
pub fn rejected(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Marker consumed by #[command_handler] / #[process_manager]; a #[saga]
    // rejects it at compile time.
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
/// Sagas are pure translators: they receive source events and produce
/// commands. Emitted commands are deferred: the router stamps their
/// `angzarr_deferred` provenance and the destination assigns the sequence.
///
/// A handler takes the event and may also declare, by name, `destinations:
/// &Destinations` (the declared output domains), `source_cover:
/// Option<Cover>` (the triggering book's cover) and `source_seq: u32` (the
/// triggering event's sequence). Process-manager handlers take
/// `destinations` and `source_cover` after their state.
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
    target: String,
    /// Audit #74: whether commands emitted to ``target`` ever use sync
    /// mode. Default ``false`` — async-only target rides the bus, no
    /// per-domain readiness probe.
    sync: bool,
}

impl syn::parse::Parse for SagaArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut source = None;
        let mut target = None;
        let mut sync: Option<bool> = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match ident.to_string().as_str() {
                "name" => {
                    let value: syn::LitStr = input.parse()?;
                    name = Some(value.value());
                }
                "source" => {
                    let value: syn::LitStr = input.parse()?;
                    source = Some(value.value());
                }
                "target" => {
                    let value: syn::LitStr = input.parse()?;
                    target = Some(value.value());
                }
                "sync" => {
                    let value: syn::LitBool = input.parse()?;
                    sync = Some(value.value);
                }
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
            sync: sync.unwrap_or(false),
        })
    }
}

/// A saga never receives a rejection: a rejected saga-emitted command is
/// compensated by the component whose event triggered the saga (its
/// `angzarr_deferred.source`). `#[rejected]` on a saga is a compile error.
fn reject_saga_compensation(input: &ItemImpl) -> Option<TokenStream2> {
    let mut errors: Option<syn::Error> = None;
    for item in &input.items {
        let ImplItem::Fn(method) = item else { continue };
        for attr in method
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("rejected"))
        {
            let e = syn::Error::new_spanned(
                attr,
                "#[rejected] is not allowed on a #[saga]: a saga never receives rejections; \
                 declare the compensation on the component whose event triggered the saga",
            );
            match errors.as_mut() {
                Some(acc) => acc.combine(e),
                None => errors = Some(e),
            }
        }
    }
    errors.map(|e| e.to_compile_error())
}

fn expand_saga(args: SagaArgs, mut input: ItemImpl) -> TokenStream2 {
    if let Some(err) = reject_saga_compensation(&input) {
        return err;
    }
    let name = &args.name;
    let source = &args.source;
    let target = &args.target;
    let sync = args.sync;

    let meta = match collect_method_metadata(&input) {
        Ok(meta) => meta,
        Err(e) => return e.to_compile_error(),
    };
    let mut regs = Vec::new();
    for (method, evt_ty) in &meta.handled_with_methods {
        let extra = match context_args(find_method(&input, method), 1, ContextKind::Saga) {
            Ok(extra) => extra,
            Err(e) => return e.to_compile_error(),
        };
        regs.push(quote! {
            let f = ::std::sync::Arc::clone(&factory);
            table = table.on_event_with_context(
                &<#evt_ty as ::prost::Name>::full_name(),
                move |any, dests, page| {
                    let _ = (&dests, &page);
                    let event: #evt_ty = __p::decode(any)?;
                    let handler = f();
                    let response = handler.#method(event #(, #extra)*).map_err(__p::rejected)?;
                    ::std::result::Result::Ok((response.commands, response.events))
                },
            );
        });
    }
    strip_method_markers(&mut input);
    let self_ty = &input.self_ty;

    let handled_exprs = meta
        .handled
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });

    quote! {
        #input

        impl ::angzarr_client::router::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::router::Kind =
                ::angzarr_client::router::Kind::Saga;
            fn handler_config() -> ::angzarr_client::router::HandlerConfig {
                ::angzarr_client::router::HandlerConfig::Saga {
                    name: #name.to_string(),
                    source: #source.to_string(),
                    target: #target.to_string(),
                    sync: #sync,
                    handled: ::std::vec![#(#handled_exprs),*],
                }
            }
            fn component(
                factory: ::angzarr_client::router::component::Factory<Self>,
            ) -> ::angzarr_client::router::component::Component {
                use ::angzarr_client::router::component as __p;
                let mut table =
                    ::angzarr_client::__router::saga::SagaDispatch::new(#name, #source, [#target]);
                #(#regs)*
                let _ = &factory;
                __p::Component::Saga(table)
            }
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                <#self_ty as ::angzarr_client::router::HandlerKind>::handler_config()
            }
        }
    }
}

/// Marks an impl block as a process manager with event handlers.
///
/// # Attributes
/// - `name = "pm-name"` - The PM's name (required)
/// - `pm_domain = "pm-domain"` - The PM's own domain for state (required)
/// - `state = StateType` - The PM's state type (required)
/// - `sources = ["domain1", "domain2"]` - Domains whose events trigger it (required)
/// - `targets = ["domain"]` - Domains it issues commands to (required)
/// - `sync_targets = ["domain"]` - Targets addressed synchronously (optional, ⊆ targets)
///
/// # Example
/// ```rust,ignore
/// #[process_manager(name = "hand-flow", pm_domain = "hand-flow", state = PMState,
///                   sources = ["table", "hand"], targets = ["hand"])]
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
    targets: Vec<String>,
    /// Audit #74: subset of ``targets`` whose commands ever use sync
    /// mode. Drives readiness probing — only sync targets get an
    /// ``OutputDomainProbe``. Default ``[]``.
    sync_targets: Vec<String>,
}

impl syn::parse::Parse for ProcessManagerArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut pm_domain = None;
        let mut state = None;
        let mut sources = None;
        let mut targets = None;
        let mut sync_targets: Option<Vec<String>> = None;
        let mut sync_targets_span: Option<proc_macro2::Span> = None;

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
                "sync_targets" => {
                    sync_targets_span = Some(ident.span());
                    sync_targets = Some(parse_str_list(input)?);
                }
                _ => return Err(syn::Error::new(ident.span(), "unknown attribute")),
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        let targets_v = require_non_empty_str_list(targets, "targets")?;
        let sync_targets_v = sync_targets.unwrap_or_default();

        // Audit #74: validate sync_targets ⊆ targets at compile time.
        let extra: Vec<&String> = sync_targets_v
            .iter()
            .filter(|t| !targets_v.contains(t))
            .collect();
        if !extra.is_empty() {
            let span = sync_targets_span.unwrap_or_else(proc_macro2::Span::call_site);
            return Err(syn::Error::new(
                span,
                format!(
                    "sync_targets {:?} are not in targets {:?}",
                    extra, targets_v
                ),
            ));
        }

        Ok(ProcessManagerArgs {
            name: require_non_empty_str(name, "name")?,
            pm_domain: require_non_empty_str(pm_domain, "pm_domain")?,
            state: state.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "state is required")
            })?,
            sources: require_non_empty_str_list(sources, "sources")?,
            targets: targets_v,
            sync_targets: sync_targets_v,
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
    let sync_targets = &args.sync_targets;

    let meta = match collect_method_metadata(&input) {
        Ok(meta) => meta,
        Err(e) => return e.to_compile_error(),
    };
    let mut regs = Vec::new();
    for (method, evt_ty) in &meta.handled_with_methods {
        let extra = match context_args(find_method(&input, method), 2, ContextKind::ProcessManager)
        {
            Ok(extra) => extra,
            Err(e) => return e.to_compile_error(),
        };
        for source in sources {
            regs.push(quote! {
                let f = ::std::sync::Arc::clone(&factory);
                table = table.on_event(
                    #source,
                    &<#evt_ty as ::prost::Name>::full_name(),
                    move |any, state, dests, source_cover| {
                        let _ = (&dests, &source_cover);
                        let event: #evt_ty = __p::decode(any)?;
                        let handler = f();
                        handler
                            .#method(event, &*state #(, #extra)*)
                            .map_err(__p::rejected)
                    },
                );
            });
        }
    }
    let rejection_regs = meta.rejected_with_methods.iter().map(|(method, d, c)| {
        let key = compensates_key(d, c);
        quote! {
            let f = ::std::sync::Arc::clone(&factory);
            table = table.on_rejected(&#key, move |notification, _rejection, state| {
                let handler = f();
                handler.#method(notification, &*state).map_err(__p::rejected)
            });
        }
    });
    strip_method_markers(&mut input);
    let self_ty = &input.self_ty;
    let rebuilder = rebuilder_expr(self_ty, state_ty, &meta);

    let handled_exprs = meta
        .handled
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let applies_exprs = meta
        .applies
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let compensates_exprs = meta
        .rejected_with_methods
        .iter()
        .map(|(_, d, c)| compensates_key(d, c));
    let state_factory_expr = match &meta.state_factory {
        Some(name) => {
            let s = name.to_string();
            quote! { ::std::option::Option::Some(#s.to_string()) }
        }
        None => quote! { ::std::option::Option::None },
    };
    let sources_vec = sources.iter().map(|s| quote! { #s.to_string() });
    let targets_vec = targets.iter().map(|s| quote! { #s.to_string() });
    let sync_targets_vec = sync_targets.iter().map(|s| quote! { #s.to_string() });

    quote! {
        #input

        impl ::angzarr_client::router::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::router::Kind =
                ::angzarr_client::router::Kind::ProcessManager;
            fn handler_config() -> ::angzarr_client::router::HandlerConfig {
                ::angzarr_client::router::HandlerConfig::ProcessManager {
                    name: #name.to_string(),
                    pm_domain: #pm_domain.to_string(),
                    sources: ::std::vec![#(#sources_vec),*],
                    targets: ::std::vec![#(#targets_vec),*],
                    sync_targets: ::std::vec![#(#sync_targets_vec),*],
                    handled: ::std::vec![#(#handled_exprs),*],
                    compensates: ::std::vec![#(#compensates_exprs),*],
                    applies: ::std::vec![#(#applies_exprs),*],
                    state_factory: #state_factory_expr,
                }
            }
            fn component(
                factory: ::angzarr_client::router::component::Factory<Self>,
            ) -> ::angzarr_client::router::component::Component {
                use ::angzarr_client::router::component as __p;
                #[allow(unused_mut)]
                let mut table = ::angzarr_client::__router::process_manager::ProcessManagerDispatch::new(
                    #name, #pm_domain, [#(#targets),*], #rebuilder,
                );
                #(#regs)*
                #(#rejection_regs)*
                let _ = &factory;
                __p::Component::ProcessManager(::std::boxed::Box::new(table))
            }
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                <#self_ty as ::angzarr_client::router::HandlerKind>::handler_config()
            }
        }
    }
}

/// Marks an impl block as a projector with event handlers.
///
/// # Attributes
/// - `name = "projector-name"` - The projector's name (required)
/// - `domains = ["domain", ...]` - Domains it consumes; `"*"` matches any (required)
///
/// # Example
/// ```rust,ignore
/// #[projector(name = "output", domains = ["player", "hand"])]
/// impl OutputProjector {
///     #[handles(PlayerRegistered)]
///     fn project_registered(&self, event: PlayerRegistered) -> CommandResult<()> {
///         // side effects through &self
///     }
///
///     #[handles(HandComplete)]
///     fn project_hand_complete(&self, event: HandComplete) -> CommandResult<()> {
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

    let meta = match collect_method_metadata(&input) {
        Ok(meta) => meta,
        Err(e) => return e.to_compile_error(),
    };
    strip_method_markers(&mut input);
    let self_ty = &input.self_ty;

    let handled_exprs = meta
        .handled
        .iter()
        .map(|ty| quote! { ::angzarr_client::full_type_url::<#ty>() });
    let domains_vec = domains.iter().map(|d| quote! { #d.to_string() });
    let domain_filter = quote! { let table = table.for_domains([#(#domains),*]); };
    let regs = meta.handled_with_methods.iter().map(|(method, evt_ty)| {
        quote! {
            let table = table.on_event(
                &<#evt_ty as ::prost::Name>::full_name(),
                |handler: &mut #self_ty, any, _page| {
                    let event: #evt_ty = __p::decode(any)?;
                    handler.#method(event).map(|_| ()).map_err(__p::rejected)
                },
            );
        }
    });

    quote! {
        #input

        impl ::angzarr_client::router::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::router::Kind =
                ::angzarr_client::router::Kind::Projector;
            fn handler_config() -> ::angzarr_client::router::HandlerConfig {
                ::angzarr_client::router::HandlerConfig::Projector {
                    name: #name.to_string(),
                    domains: ::std::vec![#(#domains_vec),*],
                    handled: ::std::vec![#(#handled_exprs),*],
                }
            }
            fn component(
                factory: ::angzarr_client::router::component::Factory<Self>,
            ) -> ::angzarr_client::router::component::Component {
                use ::angzarr_client::router::component as __p;
                // One instance per delivered book, reused across its pages.
                let table = ::angzarr_client::__router::projector::ProjectorDispatch::new(
                    #name,
                    move || factory(),
                );
                #domain_filter
                #(#regs)*
                __p::Component::Projector(::std::boxed::Box::new(table))
            }
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                <#self_ty as ::angzarr_client::router::HandlerKind>::handler_config()
            }
        }
    }
}

// Helper functions

fn get_attr_path(attr: &Attribute) -> syn::Result<syn::Path> {
    match &attr.meta {
        Meta::List(list) => syn::parse2::<syn::Path>(list.tokens.clone()).map_err(|_| {
            syn::Error::new_spanned(
                &list.tokens,
                format!(
                    "expected a message type, e.g. #[{}(MyMessage)]",
                    attr.path()
                        .get_ident()
                        .map(|i| i.to_string())
                        .unwrap_or_default()
                ),
            )
        }),
        _ => Err(syn::Error::new_spanned(
            attr,
            "expected a message type argument, e.g. #[handles(MyMessage)]",
        )),
    }
}

fn get_rejected_args(attr: &Attribute) -> syn::Result<(Option<String>, syn::Path)> {
    match &attr.meta {
        Meta::List(list) => {
            let args: RejectedArgs = syn::parse2(list.tokens.clone())
                .map_err(|e| syn::Error::new_spanned(attr, format!("#[rejected]: {e}")))?;
            Ok((args.domain, args.command))
        }
        _ => Err(syn::Error::new_spanned(
            attr,
            "expected #[rejected(command = Type)] or #[rejected(domain = \"...\", command = Type)]",
        )),
    }
}

/// `#[rejected(command = Type)]` or `#[rejected(domain = "d", command = Type)]`:
/// a `compensates` entry — the rejected command's type, optionally only
/// when it was sent to `domain`.
struct RejectedArgs {
    domain: Option<String>,
    command: syn::Path,
}

impl syn::parse::Parse for RejectedArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut domain = None;
        let mut command = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match ident.to_string().as_str() {
                "domain" => {
                    let value: syn::LitStr = input.parse()?;
                    domain = Some(require_non_empty_str(Some(value.value()), "domain")?);
                }
                "command" => {
                    command = Some(input.parse::<syn::Path>().map_err(|_| {
                        syn::Error::new(ident.span(), "command must be the rejected command's type")
                    })?);
                }
                _ => return Err(syn::Error::new(ident.span(), "unknown attribute")),
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(RejectedArgs {
            domain,
            command: command.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "command is required")
            })?,
        })
    }
}

/// The `compensates` entry expression for a `#[rejected]` method:
/// `"fq.Type"` or `"domain:fq.Type"`.
fn compensates_key(domain: &Option<String>, command: &syn::Path) -> TokenStream2 {
    match domain {
        Some(d) => quote! {
            ::std::format!("{}:{}", #d, <#command as ::prost::Name>::full_name())
        },
        None => quote! { <#command as ::prost::Name>::full_name() },
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
/// let Built::Upcaster(router) = Router::new("upcaster-player")
///     .with_handler(|| PlayerUpcaster)
///     .build()?
/// else {
///     unreachable!("a router of upcasters builds an upcaster router")
/// };
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

    let meta = match collect_method_metadata(&input) {
        Ok(meta) => meta,
        Err(e) => return e.to_compile_error(),
    };
    strip_method_markers(&mut input);
    let self_ty = &input.self_ty;

    let upcast_pairs = meta.upcasts_with_methods.iter().map(|(_, from, to)| {
        quote! {
            (
                ::angzarr_client::full_type_url::<#from>(),
                ::angzarr_client::full_type_url::<#to>(),
            )
        }
    });
    // Rules run in declaration order, each matching the event as the
    // previous rules left it.
    let regs = meta.upcasts_with_methods.iter().map(|(method, from, to)| {
        quote! {
            let table = table.on_event(&<#from as ::prost::Name>::full_name(), |any| {
                let old: #from = __p::decode(any)?;
                let new: #to = <#self_ty>::#method(old);
                ::std::result::Result::Ok(__p::pack(&new))
            });
        }
    });

    quote! {
        #input

        impl ::angzarr_client::router::HandlerKind for #self_ty {
            const KIND: ::angzarr_client::router::Kind = ::angzarr_client::router::Kind::Upcaster;
            fn handler_config() -> ::angzarr_client::router::HandlerConfig {
                ::angzarr_client::router::HandlerConfig::Upcaster {
                    name: #name.to_string(),
                    domain: #domain.to_string(),
                    upcasts: ::std::vec![#( #upcast_pairs ),*],
                }
            }
            fn component(
                factory: ::angzarr_client::router::component::Factory<Self>,
            ) -> ::angzarr_client::router::component::Component {
                use ::angzarr_client::router::component as __p;
                // Upcasts are associated functions; no instance is needed.
                let _ = factory;
                let table = ::angzarr_client::__router::upcaster::UpcasterDispatch::new(#name, #domain);
                #(#regs)*
                __p::Component::Upcaster(table)
            }
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                <#self_ty as ::angzarr_client::router::HandlerKind>::handler_config()
            }
        }
    }
}

struct UpcasterArgs {
    name: String,
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
