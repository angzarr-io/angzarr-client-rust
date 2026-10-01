//! The #[upcaster] expansion must not depend on prelude names the user
//! may shadow (Some/Ok/Err/Vec/vec!).

#![allow(non_snake_case, dead_code, unused_macros)]

use angzarr_client::upcaster;

mod shadow {
    pub struct Vec;
    pub fn Some() {}
    pub fn Ok() {}
    pub fn Err() {}
}
#[allow(unused_imports)]
use shadow::*;

macro_rules! vec {
    ($($t:tt)*) => {
        compile_error!("user vec! shadowed std")
    };
}

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
    #[upcasts(from = V1, to = V2)]
    fn up(_old: V1) -> V2 {
        V2 {}
    }
}

fn main() {}
