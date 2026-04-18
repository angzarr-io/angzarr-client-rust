//! R12 — process-manager dispatch.

use std::collections::HashMap;

use angzarr_client::proto::{
    event_page, CommandBook, Cover, EventBook, EventPage, ProcessManagerHandleRequest,
    ProcessManagerHandleResponse,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{full_type_url, process_manager, CommandResult};

use prost::Message;
use prost_types::Any;

#[derive(Clone, PartialEq, ::prost::Message)]
struct HandStarted {}
impl ::prost::Name for HandStarted {
    const NAME: &'static str = "HandStarted";
    const PACKAGE: &'static str = "hand";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct PMState {}
impl ::prost::Name for PMState {
    const NAME: &'static str = "PMState";
    const PACKAGE: &'static str = "pmg";
}

struct HandFlowPM;

#[process_manager(
    name = "pmg-hand-flow",
    pm_domain = "hand-flow",
    sources = ["hand"],
    targets = ["table"],
    state = PMState
)]
impl HandFlowPM {
    #[handles(HandStarted)]
    #[allow(unused_variables, dead_code)]
    fn on_hand_started(
        &self,
        event: HandStarted,
        state: &PMState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse {
            commands: vec![CommandBook {
                cover: Some(Cover {
                    domain: "table".to_string(),
                    ..Default::default()
                }),
                pages: vec![],
            }],
            process_events: Some(EventBook::default()),
            facts: vec![],
        })
    }
}

fn hand_started_request() -> ProcessManagerHandleRequest {
    let any = Any {
        type_url: full_type_url::<HandStarted>(),
        value: HandStarted {}.encode_to_vec(),
    };
    ProcessManagerHandleRequest {
        trigger: Some(EventBook {
            pages: vec![EventPage {
                payload: Some(event_page::Payload::Event(any)),
                ..Default::default()
            }],
            ..Default::default()
        }),
        process_state: Some(EventBook::default()),
        destination_sequences: HashMap::new(),
    }
}

#[test]
fn pm_dispatch_wraps_handler_output_into_response() {
    let built = Router::new("pmg-hand-flow")
        .with_handler(|| HandFlowPM)
        .build()
        .unwrap();
    let Built::ProcessManager(router) = built else {
        panic!("expected ProcessManager variant");
    };
    let response = router.dispatch(hand_started_request()).unwrap();
    assert_eq!(response.commands.len(), 1);
    assert_eq!(
        response.commands[0].cover.as_ref().unwrap().domain,
        "table"
    );
}
