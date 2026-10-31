//! Generated protobuf types and gRPC service definitions.
//!
//! Types are generated from `angzarr-project/proto/angzarr/*.proto` by
//! `tonic_prost_build` during `cargo build`. Includes message types
//! (Cover, EventBook, CommandBook, etc.) and gRPC client/server stubs.
//!
//! The generated code names the types of another proto package by a path
//! relative to its own package (`types.proto` imports `sererr.v1`, reached
//! as `super::super::super::sererr::v1`), so each package is included at
//! its own module path and the angzarr package re-exported here.

mod generated {
    pub mod angzarr_client {
        pub mod proto {
            pub mod angzarr {
                tonic::include_proto!("angzarr_client.proto.angzarr");
            }
        }
    }
    pub mod sererr {
        pub mod v1 {
            tonic::include_proto!("sererr.v1");
        }
    }
}

pub use generated::angzarr_client::proto::angzarr::*;
pub use generated::sererr;
