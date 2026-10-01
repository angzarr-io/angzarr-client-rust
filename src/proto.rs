//! Generated protobuf types and gRPC service definitions.
//!
//! Types are generated from `angzarr-project/proto/io/angzarr/v1/*.proto` by
//! `just generate-proto` (cross-language proto generation model — see
//! project_proto_generation_model.md). Includes message types
//! (Cover, EventBook, CommandBook, etc.) and gRPC client/server stubs.
//!
//! The included file is emitted into `src/proto/` and gitignored;
//! lefthook fires `just generate-proto` on post-checkout / post-merge.
//!
//! Module layout: prost-generated files name themselves after their proto
//! package (`io.angzarr.v1.rs`, `sererr.v1.rs`). After the
//! sererr unification (angzarr-project PR #11), `types.proto` imports
//! `sererr/v1/sererr.proto`, so the angzarr-package generated file
//! references `super::super::super::sererr::v1::CapturedError`. We honor
//! prost's nesting convention by wrapping each include in module path
//! segments matching its proto package, then flat re-export the angzarr
//! types so downstream `angzarr_client::proto::Foo` paths keep working.

#[doc(hidden)]
pub mod io {
    pub mod angzarr {
        pub mod v1 {
            include!("proto/io.angzarr.v1.rs");
        }
    }
}
pub mod sererr {
    pub mod v1 {
        include!("proto/sererr.v1.rs");
    }
}

// Flat re-export so the public surface (`angzarr_client::proto::Foo`)
// is unchanged from the pre-v1 layout.
pub use io::angzarr::v1::*;
