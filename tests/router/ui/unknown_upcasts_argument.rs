//! An `#[upcasts]` marker with an unknown argument inside an `#[upcaster]`
//! must fail to compile instead of being silently dropped.

use angzarr_client::upcaster;

#[derive(Clone, PartialEq, ::prost::Message)]
struct V1 {}
impl ::prost::Name for V1 {
    const NAME: &'static str = "V1";
    const PACKAGE: &'static str = "test";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct V2 {}
impl ::prost::Name for V2 {
    const NAME: &'static str = "V2";
    const PACKAGE: &'static str = "test";
}

struct U;

#[upcaster(name = "u", domain = "x")]
impl U {
    #[upcasts(from = V1, into = V2)]
    fn up(_old: V1) -> V2 {
        V2 {}
    }
}

fn main() {}
