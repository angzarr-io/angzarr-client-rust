//! Factory traits for per-request handler creation.
//!
//! Enables dependency injection by capturing dependencies in factory closures.
//! Each request gets a fresh handler instance with its own dependencies.

use std::sync::Arc;

/// Factory for creating handler instances per-request.
///
/// Enables dependency injection by capturing deps in the factory closure.
/// Each request gets a fresh handler with its own state.
///
/// # Example
///
/// ```rust,ignore
/// let db_pool = Arc::new(DbPool::new());
/// let factory = {
///     let db = db_pool.clone();
///     move || PlayerHandler::new(db.clone())
/// };
///
/// // Factory is called per-request to create fresh handlers
/// let router = Router::<_, CommandHandlerMode>::with_factory(factory);
/// ```
pub trait HandlerFactory<H>: Send + Sync {
    /// Create a new handler instance.
    fn create(&self) -> H;
}

// Blanket impl for closures - allows using closures directly as factories
impl<H, F> HandlerFactory<H> for F
where
    F: Fn() -> H + Send + Sync,
{
    fn create(&self) -> H {
        self()
    }
}

/// Boxed factory type for dynamic dispatch.
pub type BoxedHandlerFactory<H> = Arc<dyn HandlerFactory<H> + Send + Sync>;

/// Higher-order function factory for functional routers.
///
/// Called per-event to produce a fresh handler function.
/// Captures dependencies via closure for DI.
///
/// # Example
///
/// ```rust,ignore
/// let db = Arc::new(db_pool);
/// let router = StateRouter::<PlayerState>::new()
///     .on_with::<PlayerRegistered>(|| {
///         let db = db.clone();
///         move |state, event| {
///             // db accessible here per-event
///             state.player_id = event.email.clone();
///         }
///     });
/// ```
pub type HandlerHOF<F> = Arc<dyn Fn() -> F + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closure_as_factory() {
        let factory = || 42i32;
        let result: i32 = factory.create();
        assert_eq!(result, 42);
    }

    #[test]
    fn factory_with_captured_deps() {
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let factory = {
            let counter = counter.clone();
            move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                counter.load(std::sync::atomic::Ordering::SeqCst)
            }
        };

        assert_eq!(factory.create(), 1);
        assert_eq!(factory.create(), 2);
        assert_eq!(factory.create(), 3);
    }

    #[test]
    fn boxed_factory() {
        let factory: BoxedHandlerFactory<i32> = Arc::new(|| 99);
        assert_eq!(factory.create(), 99);
    }
}
