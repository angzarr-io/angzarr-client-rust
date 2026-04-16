//! Dispatch helpers for routing events/commands by type URL.
//!
//! Uses exact full-URL matching for type safety: `type_url == TYPE_URL_PREFIX + full_name`.
//! Pass fully-qualified proto names (e.g., `"examples.player.RegisterPlayer"`).

/// Helper macro for dispatching events by fully-qualified proto type name.
///
/// Uses exact matching: `type_url == "type.googleapis.com/" + name`.
///
/// # Example
///
/// ```rust,ignore
/// dispatch_event!(event, source, destinations, {
///     "examples.order.OrderCompleted" => self.handle_completed,
///     "examples.order.OrderCancelled" => self.handle_cancelled,
/// })
/// ```
#[macro_export]
macro_rules! dispatch_event {
    // Saga execute variant: (event, source, destinations, handlers)
    ($event:expr, $source:expr, $destinations:expr, { $($name:literal => $handler:expr),* $(,)? }) => {{
        let type_url = &$event.type_url;
        $(
            if *type_url == format!("{}{}", $crate::TYPE_URL_PREFIX, $name) {
                return $handler($source, $event, $destinations);
            }
        )*
        Ok(vec![])
    }};

    // Saga prepare variant: (event, source, handlers) -> Vec<Cover>
    ($event:expr, $source:expr, { $($name:literal => $handler:expr),* $(,)? }) => {{
        let type_url = &$event.type_url;
        $(
            if *type_url == format!("{}{}", $crate::TYPE_URL_PREFIX, $name) {
                return $handler($source, $event);
            }
        )*
        vec![]
    }};
}

/// Helper macro for dispatching commands by fully-qualified proto type name.
///
/// Uses exact matching: `type_url == "type.googleapis.com/" + name`.
///
/// # Example
///
/// ```rust,ignore
/// dispatch_command!(payload, cmd, state, seq, {
///     "examples.player.RegisterPlayer" => self.handle_register,
///     "examples.player.DepositFunds" => self.handle_deposit,
/// })
/// ```
#[macro_export]
macro_rules! dispatch_command {
    ($payload:expr, $cmd:expr, $state:expr, $seq:expr, { $($name:literal => $handler:expr),* $(,)? }) => {{
        let type_url = &$payload.type_url;
        $(
            if *type_url == format!("{}{}", $crate::TYPE_URL_PREFIX, $name) {
                return $handler($cmd, $payload, $state, $seq);
            }
        )*
        Err($crate::CommandRejectedError::new(format!("Unknown command type: {}", type_url)))
    }};
}
