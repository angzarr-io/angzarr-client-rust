//! Saga handler parameters after the event are matched by name; an
//! unknown name is a compile error rather than a silently missing value.

use angzarr_client::proto::SagaResponse;
use angzarr_client::{saga, CommandResult};

#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderCreated {}
impl ::prost::Name for OrderCreated {
    const NAME: &'static str = "OrderCreated";
    const PACKAGE: &'static str = "test";
}

struct S;

#[saga(name = "x", source = "order", target = "inventory")]
impl S {
    #[handles(OrderCreated)]
    fn on_created(&self, _evt: OrderCreated, source_seq: u32) -> CommandResult<SagaResponse> {
        let _ = source_seq;
        Ok(SagaResponse::default())
    }
}

fn main() {}
