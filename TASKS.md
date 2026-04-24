# Tasks - angzarr-client-rust

## In Progress

## To Do

- [ ] Package Publishing: publish 0.5.0 to crates.io (needs CARGO_REGISTRY_TOKEN + manual `just publish` or release workflow)

## Backlog

- [ ] Documentation: expand rustdoc on the public surface (router, handler trait, macros)
- [ ] Examples: add `examples/` directory with runnable samples (the existing `examples-rust` workspace covers this today via the cross-language examples repo, but an in-tree `examples/*.rs` would aid crate-root discovery)

## Done

- [x] Client code extracted from angzarr core repo
- [x] CI/CD: .github/workflows/ci.yml (build + test + lint + fmt + notify-downstream) and .github/workflows/mutation.yml (weekly)
- [x] Proto codegen: build.rs via prost/tonic (no buf.gen.yaml needed — Rust's codegen differs from Python's; the "update buf.gen.yaml" task was inapplicable)
- [x] Phase 1–7 cross-language parity with angzarr-client-python (see merged PRs)
