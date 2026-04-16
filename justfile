# Rust client library commands
#
# Container Overlay Pattern:
# --------------------------
# This justfile uses an overlay pattern for container execution:
#
# 1. `justfile` (this file) - runs on the host, delegates to container
# 2. `justfile.container` - mounted over this file inside the container
#
# When running outside a devcontainer:
#   - Uses pre-built angzarr-rust image from ghcr.io/angzarr
#   - Podman mounts justfile.container as /workspace/justfile
#
# When running inside a devcontainer (DEVCONTAINER=true):
#   - Commands execute directly via `just <target>`
#   - No container nesting

set shell := ["bash", "-c"]

ROOT := `git rev-parse --show-toplevel`
IMAGE := "ghcr.io/angzarr-io/angzarr-rust:latest"

# Run just target in container (or directly if already in devcontainer)
[private]
_container +ARGS:
    #!/usr/bin/env bash
    if [ "${DEVCONTAINER:-}" = "true" ]; then
        just {{ARGS}}
    else
        podman run --rm --network=host \
            -v "{{ROOT}}:/workspace:Z" \
            -v "{{ROOT}}/justfile.container:/workspace/justfile:ro" \
            -w /workspace \
            -e CARGO_HOME=/workspace/.cargo-container \
            -e DEVCONTAINER=true \
            {{IMAGE}} just {{ARGS}}
    fi

default:
    @just --list

# Generate Rust code from protos via buf
proto:
    just _container proto

# Build Rust client (release)
build:
    just _container build

# Run unit tests
test:
    just _container test

# Start gRPC test server for unified Rust harness testing
serve:
    just _container serve

# Run tests with verbose output
test-verbose:
    just _container test-verbose

# Run clippy linter
lint:
    just _container lint

# Check formatting
fmt:
    just _container fmt

# Run mutation testing with cargo-mutants (70% kill rate threshold)
mutation-test:
    just _container mutation-test

# Dry-run mutation testing (show what would be mutated)
mutation-test-dry:
    just _container mutation-test-dry

# Dry-run publish to crates.io
publish-dry:
    just _container publish-dry

# Publish to crates.io
publish:
    just _container publish

# Auto-format code
fmt-fix:
    just _container fmt-fix
