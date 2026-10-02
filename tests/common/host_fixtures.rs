//! Components, an application gRPC service and client helpers for the
//! component-host scenarios.
//!
//! The components are generic aggregates for the "order" and "payment"
//! domains: a `Record` command emits one `Recorded` event naming the
//! component that handled it, so a caller can tell which component a
//! command reached. A `Record` with `hold` set blocks its handler on a
//! [`Gate`] until the gate opens, standing in for an in-flight call.

use std::convert::Infallible;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use angzarr_client::proto::command_handler_service_client::CommandHandlerServiceClient;
use angzarr_client::proto::{
    business_response, command_page, event_page, BusinessResponse, CommandBook, CommandPage,
    ContextualCommand, Cover, EventBook, EventPage,
};
use angzarr_client::router::command_handler;
use angzarr_client::{full_type_url, CommandResult, HostAddress};
use prost::Message;
use prost_types::Any;
use tonic::codegen::{http, BoxFuture, Service};
use tonic::transport::{Channel, Endpoint, Uri};

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Record {
    #[prost(bool, tag = "1")]
    pub hold: bool,
}

impl ::prost::Name for Record {
    const PACKAGE: &'static str = "test.host";
    const NAME: &'static str = "Record";
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Recorded {
    #[prost(string, tag = "1")]
    pub component: String,
}

impl ::prost::Name for Recorded {
    const PACKAGE: &'static str = "test.host";
    const NAME: &'static str = "Recorded";
}

/// A latch the handlers wait on while a call is held in flight.
#[derive(Debug, Default)]
pub struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
    held: Mutex<u32>,
}

impl Gate {
    pub fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }

    /// Calls currently blocked on the gate.
    pub fn held(&self) -> u32 {
        *self.held.lock().unwrap()
    }

    fn wait(&self) {
        *self.held.lock().unwrap() += 1;
        let mut open = self.open.lock().unwrap();
        while !*open {
            let (guard, timeout) = self
                .changed
                .wait_timeout(open, Duration::from_secs(10))
                .unwrap();
            open = guard;
            if timeout.timed_out() {
                break;
            }
        }
        drop(open);
        *self.held.lock().unwrap() -= 1;
    }
}

#[derive(Default)]
pub struct Tally;

fn record(component: &str, cmd: &Record, gate: &Gate) -> EventBook {
    if cmd.hold {
        gate.wait();
    }
    EventBook {
        pages: vec![EventPage {
            payload: Some(event_page::Payload::Event(Any {
                type_url: full_type_url::<Recorded>(),
                value: Recorded {
                    component: component.into(),
                }
                .encode_to_vec(),
            })),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Aggregate component for the "order" domain.
pub struct OrderComponent(pub Arc<Gate>);

#[command_handler(domain = "order", state = Tally)]
impl OrderComponent {
    #[handles(Record)]
    fn on_record(&self, cmd: Record, _state: &Tally, _seq: u32) -> CommandResult<EventBook> {
        Ok(record("order", &cmd, &self.0))
    }
}

/// Aggregate component for the "payment" domain.
pub struct PaymentComponent(pub Arc<Gate>);

#[command_handler(domain = "payment", state = Tally)]
impl PaymentComponent {
    #[handles(Record)]
    fn on_record(&self, cmd: Record, _state: &Tally, _seq: u32) -> CommandResult<EventBook> {
        Ok(record("payment", &cmd, &self.0))
    }
}

/// An application-defined gRPC service with one unary method,
/// `Echo(Cover) -> Cover`, answering with the domain prefixed by "report:".
#[derive(Clone, Default)]
pub struct OrderReportService;

pub const ORDER_REPORT_SERVICE: &str = "test.host.OrderReportService";
const ECHO_PATH: &str = "/test.host.OrderReportService/Echo";

impl tonic::server::NamedService for OrderReportService {
    const NAME: &'static str = ORDER_REPORT_SERVICE;
}

struct Echo;

impl tonic::server::UnaryService<Cover> for Echo {
    type Response = Cover;
    type Future = BoxFuture<tonic::Response<Cover>, tonic::Status>;

    fn call(&mut self, request: tonic::Request<Cover>) -> Self::Future {
        Box::pin(async move {
            Ok(tonic::Response::new(Cover {
                domain: format!("report:{}", request.into_inner().domain),
                ..Default::default()
            }))
        })
    }
}

impl Service<http::Request<tonic::body::Body>> for OrderReportService {
    type Response = http::Response<tonic::body::Body>;
    type Error = Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
        Box::pin(async move {
            if req.uri().path() == ECHO_PATH {
                let mut grpc = tonic::server::Grpc::new(tonic_prost::ProstCodec::default());
                Ok(grpc.unary(Echo, req).await)
            } else {
                Ok(tonic::Status::unimplemented("").into_http())
            }
        })
    }
}

/// A channel to a running host, over TCP or its Unix socket.
pub async fn connect(address: &HostAddress) -> Channel {
    match address {
        HostAddress::Tcp(addr) => {
            let host = match addr {
                std::net::SocketAddr::V4(_) => addr.to_string(),
                std::net::SocketAddr::V6(a) if a.ip().is_unspecified() => {
                    format!("[::1]:{}", a.port())
                }
                std::net::SocketAddr::V6(_) => addr.to_string(),
            };
            Endpoint::from_shared(format!("http://{host}"))
                .expect("endpoint")
                .connect()
                .await
                .expect("connect over tcp")
        }
        HostAddress::Uds(path) => {
            let path = path.clone();
            Endpoint::try_from("http://[::]:50051")
                .expect("endpoint")
                .connect_with_connector(tower::service_fn(move |_: Uri| {
                    let path = path.clone();
                    async move {
                        tokio::net::UnixStream::connect(path)
                            .await
                            .map(hyper_util::rt::TokioIo::new)
                    }
                }))
                .await
                .expect("connect over unix socket")
        }
    }
}

/// A `Record` command for `domain`.
pub fn record_command(domain: &str, hold: bool) -> ContextualCommand {
    ContextualCommand {
        command: Some(CommandBook {
            cover: Some(Cover {
                domain: domain.into(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                payload: Some(command_page::Payload::Command(Any {
                    type_url: full_type_url::<Record>(),
                    value: Record { hold }.encode_to_vec(),
                })),
                ..Default::default()
            }],
        }),
        events: None,
    }
}

/// Which component handled a `Record` call, from its `Recorded` event.
pub fn handled_by(response: &BusinessResponse) -> String {
    let Some(business_response::Result::Events(book)) = &response.result else {
        panic!("expected events, got {response:?}");
    };
    let Some(event_page::Payload::Event(any)) = book.pages.first().and_then(|p| p.payload.clone())
    else {
        panic!("expected one event, got {book:?}");
    };
    Recorded::decode(any.value.as_slice())
        .expect("Recorded")
        .component
}

/// Send a `Record` for `domain` to the host's CommandHandlerService.
pub async fn send_record(
    channel: Channel,
    domain: &str,
    hold: bool,
) -> Result<BusinessResponse, tonic::Status> {
    CommandHandlerServiceClient::new(channel)
        .handle(record_command(domain, hold))
        .await
        .map(|r| r.into_inner())
}

/// Call `OrderReportService.Echo`.
pub async fn echo(channel: Channel, domain: &str) -> Result<Cover, tonic::Status> {
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready()
        .await
        .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
    grpc.unary(
        tonic::Request::new(Cover {
            domain: domain.into(),
            ..Default::default()
        }),
        http::uri::PathAndQuery::from_static(ECHO_PATH),
        tonic_prost::ProstCodec::default(),
    )
    .await
    .map(|r| r.into_inner())
}

/// The health status the host reports for `service` (`""` is the overall
/// server), or the gRPC error code when the check fails.
pub async fn health(
    channel: Channel,
    service: &str,
) -> Result<tonic_health::pb::health_check_response::ServingStatus, tonic::Code> {
    let mut client = tonic_health::pb::health_client::HealthClient::new(channel);
    client
        .check(tonic_health::pb::HealthCheckRequest {
            service: service.into(),
        })
        .await
        .map(|r| r.into_inner().status())
        .map_err(|s| s.code())
}

/// Poll health until it reports `want` for `service` or two seconds pass.
pub async fn await_health(
    channel: Channel,
    service: &str,
    want: tonic_health::pb::health_check_response::ServingStatus,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        if health(channel.clone(), service).await == Ok(want) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}
