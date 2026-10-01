//! A saga never receives rejections (C-0484): #[rejected] on a #[saga]
//! must fail to compile.

use angzarr_client::proto::{Notification, SagaResponse};
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
    fn on_created(&self, _evt: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse::default())
    }

    #[rejected(domain = "inventory", command = ReserveStock)]
    fn on_rejected(&self, _n: &Notification) -> CommandResult<SagaResponse> {
        Ok(SagaResponse::default())
    }
}

fn main() {}
