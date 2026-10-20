//! Procedural macros for angzarr OO-style component definitions.
//!
//! # Command-handler Example
//!
//! ```rust,ignore
//! use angzarr_client::router::{applies, command_handler, handles};
//!
//! #[command_handler(domain = "counter", state = CounterState)]
//! impl Counter {
//!     #[applies(Increased)]
//!     fn apply_increased(state: &mut CounterState, event: Increased) {
//!         state.value += event.by;
//!     }
//!
//!     #[handles(Increase)]
//!     fn increase(&self, cmd: Increase, state: &CounterState, seq: u64)
//!         -> CommandResult<EventBook> {
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
/// #[command_handler(domain = "counter", state = CounterState)]
/// impl Counter {
///     #[handles(Increase)]
///     fn register(&self, cmd: Increase, state: &CounterState, seq: u64)
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

    let config_expr: TokenStream2 = quote! {
        ::angzarr_client::router::HandlerConfig::CommandHandler {
            domain: #domain.to_string(),
            handled: ::std::vec![#(#handled_exprs),*],
            applies: ::std::vec![#(#applies_exprs),*],
            state_factory: #state_factory_expr,
            handles_fact: ::std::vec![#(#handles_fact_exprs),*],
        }
    };
    // An aggregate is described, and coordinated, by its domain
    // (handler-contract C-0565: the "order" aggregate).
    let name = domain;

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
                rules_version: &str,
            ) -> ::angzarr_client::router::component::Component {
                use ::angzarr_client::router::component as __p;
                #[allow(unused_mut)]
                let mut table = ::angzarr_client::router::binding::aggregate::AggregateDispatch::new(
                    #name, #domain, #rebuilder,
                )
                .with_rules_version(rules_version);
                #(#command_regs)*
                #(#fact_regs)*
                let _ = &factory;
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
/// `#[state_factory]` (or `Default`), every `#[applies]` method, and, when
/// `S` is a protobuf message, its state type, snapshot image and snapshot
/// loader.
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
        use ::angzarr_client::router::component::{DeclaresMessageState as _, OpaqueState as _};
        let rebuilder = ::angzarr_client::router::binding::rebuild::Rebuilder::<#state_ty>::new(|| #initial);
        #(#appliers)*
        (&::angzarr_client::router::component::StateMessage::<#state_ty>::new()).declare(rebuilder)
    }}
}

/// Which component a handler's optional context parameters come from.
#[derive(Clone, Copy, PartialEq)]
enum ContextKind {
    /// The triggering `page` context is in scope.
    Saga,
    /// `source_cover` is in scope.
    ProcessManager,
}

/// The optional context parameters a saga / process-manager handler may
/// declare after its required ones, by name: `source_cover` (the
/// triggering book's cover) and, for a saga, `source_seq` (the triggering
/// event's sequence).
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
                         `source_cover` and `source_seq`"
                    ),
                ))
            }
            (other, ContextKind::ProcessManager) => {
                return Err(syn::Error::new_spanned(
                    &ident.ident,
                    format!(
                        "unsupported handler parameter `{other}`: optional parameters are \
                         `source_cover`"
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
    /// `(method name, fact type)` pairs for fact-dispatch
    /// dispatch-arm generation.
    handles_fact_with_methods: Vec<(Ident, syn::Path)>,
}

/// Method-level markers a kind macro consumes. A method carries at most one
/// of them: a handler, fact handler, applier, state factory and upcast are
/// different roles with different signatures.
const METHOD_MARKERS: &[&str] = &[
    "handles",
    "handles_fact",
    "applies",
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

/// Strip `#[handles]`, `#[handles_fact]`, `#[applies]`, `#[state_factory]`,
/// `#[upcasts]` from the methods of an impl block so rustc doesn't see
/// them as unknown attrs.
fn strip_method_markers(input: &mut ItemImpl) {
    for item in &mut input.items {
        if let ImplItem::Fn(method) = item {
            method.attrs.retain(|attr| {
                !attr.path().is_ident("handles")
                    && !attr.path().is_ident("handles_fact")
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
/// #[handles(Increase)]
/// fn register(&self, cmd: Increase, state: &CounterState, seq: u64)
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
/// reality, e.g. a payment confirmation) in the Fact arm of a command page
/// delivered to `CommandHandlerService.Handle`. The
/// method receives `(self, fact, state)` after state has been rebuilt
/// from prior events. It returns the fact to record, optionally followed by
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
/// #[applies(Increased)]
/// fn apply_registered(state: &mut CounterState, event: Increased) {
///     state.value += event.by;
///     state.display_name = event.display_name;
///     state.exists = true;
/// }
///
/// #[applies(FundsDeposited)]
/// fn apply_deposited(state: &mut CounterState, event: FundsDeposited) {
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
/// commands. Emitted commands pass through as the handler built them: the
/// coordinator stamps their identity, causation and correlation, and the
/// destination stamps the sequence.
///
/// A handler takes the event and may also declare, by name, `source_cover:
/// Option<Cover>` (the triggering book's cover) and `source_seq: u64` (the
/// triggering event's sequence). Process-manager handlers take
/// `source_cover` after their state.
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
///         // Build commands with the target cover set (the coordinator stamps the rest)
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
}

impl syn::parse::Parse for SagaArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut source = None;
        let mut target = None;

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
                move |any, page| {
                    let _ = &page;
                    let event: #evt_ty = __p::decode(any)?;
                    let handler = f();
                    let response = handler.#method(event #(, #extra)*).map_err(__p::rejected)?;
                    ::std::result::Result::Ok(response.commands)
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
                    handled: ::std::vec![#(#handled_exprs),*],
                }
            }
            fn component(
                factory: ::angzarr_client::router::component::Factory<Self>,
                rules_version: &str,
            ) -> ::angzarr_client::router::component::Component {
                use ::angzarr_client::router::component as __p;
                let mut table =
                    ::angzarr_client::router::binding::saga::SagaDispatch::new(#name, #source, [#target])
                        .with_rules_version(rules_version);
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
                    move |any, state, source_cover| {
                        let _ = &source_cover;
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
    let state_factory_expr = match &meta.state_factory {
        Some(name) => {
            let s = name.to_string();
            quote! { ::std::option::Option::Some(#s.to_string()) }
        }
        None => quote! { ::std::option::Option::None },
    };
    let sources_vec = sources.iter().map(|s| quote! { #s.to_string() });
    let targets_vec = targets.iter().map(|s| quote! { #s.to_string() });

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
                    handled: ::std::vec![#(#handled_exprs),*],
                            applies: ::std::vec![#(#applies_exprs),*],
                    state_factory: #state_factory_expr,
                }
            }
            fn component(
                factory: ::angzarr_client::router::component::Factory<Self>,
                rules_version: &str,
            ) -> ::angzarr_client::router::component::Component {
                use ::angzarr_client::router::component as __p;
                #[allow(unused_mut)]
                let mut table = ::angzarr_client::router::binding::process_manager::ProcessManagerDispatch::new(
                    #name, #pm_domain, [#(#targets),*], #rebuilder,
                )
                .with_rules_version(rules_version);
                #(#regs)*
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
/// #[projector(name = "output", domains = ["counter", "ledger"])]
/// impl OutputProjector {
///     #[handles(Increased)]
///     fn project_registered(&self, event: Increased) -> CommandResult<()> {
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
                rules_version: &str,
            ) -> ::angzarr_client::router::component::Component {
                use ::angzarr_client::router::component as __p;
                // One instance per delivered book, reused across its pages.
                let table = ::angzarr_client::router::binding::projector::ProjectorDispatch::new(
                    #name,
                    move || factory(),
                )
                .with_rules_version(rules_version);
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
/// #[upcasts(from = IncreasedV1, to = IncreasedV2)]
/// fn upgrade(old: IncreasedV1) -> IncreasedV2 {
///     IncreasedV2 { /* ... */ }
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

/// Marks an impl as an upcaster chain — transforms events in a given
/// `domain` from one version to another. A chain is part of its component,
/// never a component itself: give it to the component's router with
/// `Router::with_upcaster`, and the component's host serves
/// `UpcasterService` alongside the component.
///
/// # Attributes
/// - `name = "..."` — identifier for this upcaster (used in logging / registry)
/// - `domain = "..."` — the aggregate domain whose events this upcaster covers
///
/// # Example
/// ```rust,ignore
/// #[upcaster(name = "counter-v1-to-v2", domain = "counter")]
/// impl CounterUpcaster {
///     #[upcasts(from = IncreasedV1, to = IncreasedV2)]
///     fn upgrade(old: IncreasedV1) -> IncreasedV2 { /* ... */ }
/// }
///
/// let built = Router::new("counter")
///     .with_handler(|| Counter)
///     .with_upcaster(|| CounterUpcaster)
///     .build()?;
/// ```
///
/// The macro emits `impl UpcasterKind` + `impl Handler` on the annotated
/// type, so `Router::with_upcaster` accepts it and `with_handler` (which
/// takes components) does not. Dispatch matches each incoming
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

        impl ::angzarr_client::router::UpcasterKind for #self_ty {
            fn handler_config() -> ::angzarr_client::router::HandlerConfig {
                ::angzarr_client::router::HandlerConfig::Upcaster {
                    name: #name.to_string(),
                    domain: #domain.to_string(),
                    upcasts: ::std::vec![#( #upcast_pairs ),*],
                }
            }
            fn upcaster() -> ::angzarr_client::router::binding::upcaster::UpcasterDispatch {
                use ::angzarr_client::router::component as __p;
                // Upcasts are associated functions; no instance is needed.
                let table = ::angzarr_client::router::binding::upcaster::UpcasterDispatch::new(#name, #domain);
                #(#regs)*
                table
            }
        }

        impl ::angzarr_client::router::Handler for #self_ty {
            fn config(&self) -> ::angzarr_client::router::HandlerConfig {
                <#self_ty as ::angzarr_client::router::UpcasterKind>::handler_config()
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
