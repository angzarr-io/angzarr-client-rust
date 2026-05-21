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

# Reusable submodule-protection recipes (install-submodule-hooks,
# check-submodules-clean). Source of truth: angzarr-project/submodule.just.
import? 'angzarr-project/submodule.just'

ROOT := `git rev-parse --show-toplevel`
IMAGE := "ghcr.io/angzarr-io/angzarr-rust:latest"

# Run just target in container (or directly if already in devcontainer)
[private]
_container +ARGS:
    #!/usr/bin/env bash
    if [ "${DEVCONTAINER:-}" = "true" ]; then
        just {{ARGS}}
    else
        docker run --rm --network=host \
            -v "{{ROOT}}:/workspace:Z" \
            -v "{{ROOT}}/justfile.container:/workspace/justfile:ro" \
            -w /workspace \
            -e CARGO_HOME=/workspace/.cargo-container \
            -e DEVCONTAINER=true \
            {{IMAGE}} just {{ARGS}}
    fi

# Run a mutation-testing target with the workspace mounted READ-ONLY.
#
# WHY:
#   cargo-mutants --in-place writes mutated source into the working tree. If
#   the workspace is bind-mounted RW (as `_container` does) and the container
#   dies mid-run, the mutated files are left on the host. This helper closes
#   that hole: source is mounted at /src:ro, a tar-piped copy lands in /work
#   inside the container's WRITABLE OVERLAY LAYER, and `--rm` destroys the
#   overlay (and the mutated copy) on every exit.
#
# WHAT TOUCHES THE HOST:
#   - {{ROOT}}/.mutants-cache/cargo-{home,target} — compiled artifacts and
#     dep registry only. NEVER contains mutated source files. Gitignored.
#     Delete the dir to purge the cache.
#   - {{ROOT}}/mutants.out/outcomes.json — copied out at the end of a
#     successful run so external tooling can read it.
#
# WHAT NEVER TOUCHES THE HOST:
#   - Mutated source trees (live in /work, container overlay, --rm wipes).
#   - cargo-mutants's intermediate working dirs.
[private]
_container-ephemeral +ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "${DEVCONTAINER:-}" = "true" ]; then
        # Already inside a devcontainer — that container IS the ephemeral
        # boundary. Run directly; the outer just wrapper ensures --rm.
        just --justfile "{{ROOT}}/justfile.container" {{ARGS}}
        exit 0
    fi
    mkdir -p "{{ROOT}}/mutants.out" \
             "{{ROOT}}/.mutants-cache/cargo-home" \
             "{{ROOT}}/.mutants-cache/cargo-target"
    docker run --rm --network=host \
        -v "{{ROOT}}:/src:ro,Z" \
        -v "{{ROOT}}/mutants.out:/out:Z" \
        -v "{{ROOT}}/.mutants-cache/cargo-home:/cargo-home:Z" \
        -v "{{ROOT}}/.mutants-cache/cargo-target:/cargo-target:Z" \
        -v "{{ROOT}}/justfile.container:/etc/angzarr-justfile:ro" \
        -e CARGO_HOME=/cargo-home \
        -e CARGO_TARGET_DIR=/cargo-target \
        -e DEVCONTAINER=true \
        -e MUTANTS_EPHEMERAL=1 \
        -e MUTANTS_OUT_DIR=/out \
        -w /work \
        {{IMAGE}} bash -eu -o pipefail -c '
            # Self-heal: install cargo-mutants on demand if the image
            # does not ship it. Cached in /cargo-home across runs.
            if ! command -v cargo-mutants >/dev/null; then
                echo "[ephemeral] cargo-mutants missing from image; installing to cached CARGO_HOME"
                cargo install cargo-mutants --locked
            fi
            echo "[ephemeral] copying /src -> /work (container overlay)"
            mkdir -p /work
            # tar|tar: rsync is not in the base image. Excludes mirror what
            # rsync would skip — build artifacts, prior mutation output,
            # host-side cargo caches, and the new mutants cache itself.
            tar -C /src \
                --exclude=./target \
                --exclude=./.cargo-container \
                --exclude=./.mutants-cache \
                --exclude=./mutants.out \
                --exclude=./mutants.out.old \
                -cf - . \
                | tar -C /work -xf -
            # Mount the container-side justfile into the copy so `just` finds
            # it (the original /src is read-only, but /work is writable).
            cp /etc/angzarr-justfile /work/justfile
            cd /work
            just {{ARGS}}
            # Persist ONLY outcomes.json back to host. Mutated source trees
            # and intermediate working dirs die with the container.
            if [ -f /work/mutants.out/outcomes.json ]; then
                cp /work/mutants.out/outcomes.json /out/outcomes.json
                echo "[ephemeral] outcomes.json copied to host mutants.out/"
            fi
        '

default:
    @just --list

# =============================================================================
# Proto generation — cross-language model (project_proto_generation_model)
# =============================================================================
# `.proto` sources live in the angzarr-project submodule. Bindings are NEVER
# committed (see .gitignore: src/proto/*.rs). They are regenerated:
#   1. on `post-checkout` / `post-merge` via lefthook (covers fresh clones,
#      branch switches, submodule bumps)
#   2. transparently as a recipe dependency of `build`, `test`, `lint`, etc.
#      The recipe is idempotent — mtime guard skips when bindings are newer
#      than the newest .proto source.
#
# Runs in the same devcontainer image used for build/test/mutation so the
# protoc + tonic_prost_build toolchain is fixed (no host fallback). Rootless
# docker requires `-u 0:0` per feedback_docker_rootless.
#
# Build-tool integration (build.rs) is intentionally NOT the regen trigger:
# build.rs only runs codegen when GENERATE_PROTOS=1 is set, which this
# recipe sets. Plain `cargo build` consumes the pre-emitted src/proto/*.rs
# files via `include!` in src/proto.rs. This keeps the regen orchestration
# consistent across the 6-lang ecosystem.

PROTO_SRC_DIR := ROOT + "/angzarr-project/proto"
PROTO_OUT_DIR := ROOT + "/src/proto"

# Public entry point. Idempotent: returns immediately if bindings are
# fresher than the newest .proto source.
generate-proto:
    #!/usr/bin/env bash
    set -euo pipefail
    src_dir="{{PROTO_SRC_DIR}}"
    out_dir="{{PROTO_OUT_DIR}}"
    if [ ! -d "$src_dir" ]; then
        echo "[generate-proto] $src_dir missing — is the angzarr-project submodule initialized?" >&2
        exit 1
    fi
    # Staleness check: regenerate if any .proto file is newer than the
    # OLDEST generated binding, or if no bindings exist yet.
    # Catches "submodule bumped" and "fresh clone" — the hot paths driving
    # the lefthook trigger. Does NOT catch manual deletion of one binding
    # while others remain fresh; use `just generate-proto-force` for that.
    newest_proto=$(find "$src_dir" -name '*.proto' -printf '%T@\n' 2>/dev/null \
                    | sort -n | tail -1)
    if [ -d "$out_dir" ]; then
        oldest_pb=$(find "$out_dir" -name '*.rs' -printf '%T@\n' 2>/dev/null \
                        | sort -n | head -1)
    else
        oldest_pb=""
    fi
    if [ -n "$newest_proto" ] && [ -n "$oldest_pb" ] \
        && awk -v p="$newest_proto" -v b="$oldest_pb" 'BEGIN{exit !(b>p)}'; then
        echo "[generate-proto] bindings up-to-date, skipping (use 'just generate-proto-force' to override)"
        exit 0
    fi
    just generate-proto-force

# Always regenerate, ignoring mtimes. Invoked by `generate-proto` when stale
# and exposed directly for users who want to force a rebuild.
generate-proto-force:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "${DEVCONTAINER:-}" = "true" ]; then
        # Inside the devcontainer image already — run directly.
        just --justfile "{{ROOT}}/justfile.container" generate-proto-force
        exit 0
    fi
    # Rootless docker: -u 0:0 maps to host user via subuid; writes to the
    # bind-mount land owned by the host user. Rootful: direct uid match.
    # See feedback_docker_rootless.
    if docker info --format '{{{{.SecurityOptions}}}}' 2>/dev/null | grep -q rootless; then
        USER_FLAG="-u 0:0"
    else
        USER_FLAG="-u $(id -u):$(id -g)"
    fi
    docker run --rm --network=host \
        $USER_FLAG \
        -v "{{ROOT}}:/workspace:Z" \
        -v "{{ROOT}}/justfile.container:/workspace/justfile:ro" \
        -w /workspace \
        -e CARGO_HOME=/workspace/.cargo-container \
        -e DEVCONTAINER=true \
        {{IMAGE}} just generate-proto-force

# Build Rust client (release)
build: generate-proto
    just _container build

# Run unit tests
test: generate-proto
    just _container test

# Start gRPC test server for unified Rust harness testing
serve: generate-proto
    just _container serve

# Run tests with verbose output
test-verbose: generate-proto
    just _container test-verbose

# Run clippy linter
lint: generate-proto
    just _container lint

# Check formatting
fmt: generate-proto
    just _container fmt

# Auto-format code
fmt-fix: generate-proto
    just _container fmt-fix

# Cross-language alias — `just check` runs lint + fmt-check.
check: lint fmt

# Remove build artifacts
clean:
    just _container clean

# =============================================================================
# Mutation Testing
# =============================================================================
# All cargo-mutants runs go through `_container-ephemeral` so the mutated
# source lives in the container's writable overlay layer and is destroyed
# with `--rm`. Running cargo-mutants on the host is FORBIDDEN.
# =============================================================================

# Run mutation testing with cargo-mutants (70% kill rate threshold).
# Ephemeral: source mounted read-only, mutations live in container overlay.
mutation-test: generate-proto
    just _container-ephemeral mutation-test

# Dry-run mutation testing (show what would be mutated). Also ephemeral.
mutation-test-dry: generate-proto
    just _container-ephemeral mutation-test-dry

# Purge local mutation build cache (compiled artifacts only; no mutated source)
mutants-purge-cache:
    rm -rf "{{ROOT}}/.mutants-cache"
    @echo "Removed {{ROOT}}/.mutants-cache"

# Dry-run publish to crates.io
publish-dry: generate-proto
    just _container publish-dry

# Publish to crates.io
publish: generate-proto
    just _container publish

# ---------------------------------------------------------------------------
# Submodule safety — mirrors Python's `submodules-lock` / `submodules-unlock`
# / `bump-angzarr-project` recipes. Content edits to angzarr-project must go
# through the super-repo; these targets enforce the lock/unlock/bump pattern.
# ---------------------------------------------------------------------------

# Lock angzarr-project read-only (filesystem enforcement against stray edits).
submodules-lock:
    chmod -R a-w angzarr-project

# Unlock angzarr-project for manual edits. Remember to `submodules-lock` after.
submodules-unlock:
    chmod -R u+w angzarr-project

# Bump angzarr-project to latest on its tracking branch.
# Unlock → update from remote → re-stage pointer → re-lock.
bump-angzarr-project:
    chmod -R u+w angzarr-project
    git submodule update --remote --merge angzarr-project
    git add angzarr-project
    chmod -R a-w angzarr-project
