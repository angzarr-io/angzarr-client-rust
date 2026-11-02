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

# Rootless docker maps container uid 0 to the host user, so `-u 0:0` is what
# makes bind-mounted writes (target/, .cargo-container/) land as the host
# user; the image's default user maps to an unowned subuid there. Rootful
# docker keeps the image default.
DOCKER_USER := if `docker info 2>/dev/null | grep -q rootless && echo yes || echo no` == "yes" { "-u 0:0" } else { "" }

# Run just target in container (or directly if already in devcontainer)
[private]
_container +ARGS:
    #!/usr/bin/env bash
    if [ "${DEVCONTAINER:-}" = "true" ]; then
        # Name the container justfile explicitly. A bare `just {{ARGS}}`
        # resolves to THIS (host) justfile, whose recipes call `_container`
        # again: build -> _container build -> just build -> ... with no
        # limit. That recursion exhausted the cgroup's task cap and killed
        # the session running it (2026-10-26). `_container-ephemeral`
        # already does this.
        just --justfile "{{ROOT}}/justfile.container" {{ARGS}}
    else
        # Mount the shared git dir at its host path so linked worktrees
        # (whose .git file points there) resolve inside the container.
        git_common="$(git rev-parse --path-format=absolute --git-common-dir)"
        docker run --rm --network=host {{DOCKER_USER}} \
            -v "{{ROOT}}:/workspace:Z" \
            -v "${git_common}:${git_common}" \
            -v "{{ROOT}}/justfile.container:/workspace/justfile:ro" \
            -w /workspace \
            -e CARGO_HOME=/workspace/.cargo-container \
            -e DEVCONTAINER=true \
            -e GIT_CONFIG_COUNT=1 \
            -e GIT_CONFIG_KEY_0=safe.directory \
            -e GIT_CONFIG_VALUE_0='*' \
            {{IMAGE}} just {{ARGS}}
    fi

default:
    @just --list

# Build Rust client (release)
build:
    just _container build

# Run unit tests
test:
    just _container test

# Build the tests without running them
build-tests:
    just _container build-tests

# lefthook's pre-commit and pre-merge-commit tests, under the rule: red tests
# are allowed on branches and never merged red. The tests are built first (a
# build failure is refused everywhere). Red tests (the tests build and some
# fail) are committed on a branch other than the default one, with a warning
# that it cannot merge until they pass, and refused on the default branch or
# a detached HEAD; `push-check` (pre-push) keeps them out of the default
# branch. The default branch is origin/HEAD, else main.
test-commit:
    #!/usr/bin/env bash
    set -euo pipefail
    cd '{{ROOT}}'
    just --justfile '{{ROOT}}/justfile' build-tests
    just --justfile '{{ROOT}}/justfile' test && exit 0
    default=$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null) && default=${default#origin/} || default=main
    branch=$(git symbolic-ref --quiet --short HEAD) || branch=
    if [ -n "$branch" ] && [ "$branch" != "$default" ]; then
        echo "RED  tests: red tests committed on $branch, which is allowed off $default; $branch cannot merge into $default (or be pushed to it) until they pass"
        exit 0
    fi
    echo "FAIL tests: red tests cannot be committed on ${branch:-a detached HEAD}. Commit them on a branch." >&2
    exit 1

# lefthook's pre-push step, given the remote's name and git's pushed refs on
# stdin: each pushed commit's tests (`test`), run in that commit's own tree
# (target/push-tree, a detached worktree), whatever is or is not staged or
# checked out here. A deleted ref passes at once. The push tree's
# angzarr-project is the pushed commit's submodule commit, exported from this
# checkout's submodule (fetched if it is missing there). The run's log is
# target/logs/push-tests.log.
#
# Red tests are allowed on a branch and kept out of the default branch
# (<remote>/HEAD, else main): pushed to another branch, failing tests pass
# with a warning that the branch cannot merge until they pass; pushed to the
# default branch they are refused. `lefthook` is what lefthook.yml passes to
# stop lefthook skipping it, and is ignored. To check a push to main by hand
# without pushing:
#
#     echo "refs/heads/B $(git rev-parse HEAD) refs/heads/main 0" | just push-check origin
[positional-arguments]
push-check remote *lefthook:
    #!/usr/bin/env bash
    set -euo pipefail
    # git exports repository-locating variables (GIT_DIR, ...) into a hook;
    # with them, a git -C the push tree or a submodule would act on this
    # checkout instead.
    unset $(git rev-parse --local-env-vars)
    top='{{ROOT}}'
    remote=$1
    default=$(git -C "$top" symbolic-ref --quiet --short "refs/remotes/$remote/HEAD" 2>/dev/null) && default=${default#"$remote"/} || default=main
    tree="$top/target/push-tree"
    mkdir -p "$top/target/logs"
    log="$top/target/logs/push-tests.log"
    exec 9> "$top/target/.push-tree.lock"
    flock 9
    pushed=0
    while read -r local_ref local_sha remote_ref remote_sha; do
        [ -n "${local_sha:-}" ] || continue
        [ "${local_sha//0/}" = "" ] && continue  # a deletion
        pushed=1
        target=${remote_ref#refs/heads/}
        base=
        if [ -n "${remote_sha//0/}" ] && git -C "$top" cat-file -e "$remote_sha^{commit}" 2>/dev/null; then
            base=$(git -C "$top" merge-base "$remote_sha" "$local_sha") || base=
        fi
        if [ -z "$base" ]; then
            for ref in "refs/remotes/$remote/$default" "refs/heads/$default"; do
                if git -C "$top" rev-parse --verify --quiet "$ref^{commit}" > /dev/null; then
                    base=$(git -C "$top" merge-base "$ref" "$local_sha") && break
                fi
            done
        fi
        if [ -z "$base" ]; then
            echo "FAIL push-check: $local_ref shares no history with $remote's tip or with $default" >&2
            exit 1
        fi
        range="$(git -C "$top" rev-parse --short "$base")..$(git -C "$top" rev-parse --short "$local_sha") to $remote/$target"
        # The pushed commit's tree, alone; no hook of this repository runs for
        # it (core.hooksPath), and its target/ and .cargo-container/ (kept by
        # clean) carry builds across pushes.
        nohooks=(-c core.hooksPath=/dev/null)
        if ! git -C "$tree" rev-parse --git-dir > /dev/null 2>&1; then
            rm -rf "$tree"
            git -C "$top" worktree prune
            git "${nohooks[@]}" -C "$top" worktree add --quiet --detach "$tree" "$local_sha"
        fi
        git "${nohooks[@]}" -C "$tree" checkout --quiet --detach --force "$local_sha"
        git -C "$tree" clean -fdq -e /target/ -e /.cargo-container/
        sub=$(git -C "$top" ls-tree "$local_sha" angzarr-project | awk '{print $3}')
        if ! git -C "$top/angzarr-project" cat-file -e "$sub^{commit}" 2>/dev/null; then
            git -C "$top/angzarr-project" fetch --quiet origin "$sub" || {
                echo "FAIL push-check: angzarr-project $sub, the pushed commit's submodule, is not here (git submodule update --init)" >&2
                exit 1
            }
        fi
        rm -rf "$tree/angzarr-project"
        mkdir -p "$tree/angzarr-project"
        git -C "$top/angzarr-project" archive "$sub" | tar -x -C "$tree/angzarr-project"
        rc=0
        (cd "$tree" && just --justfile "$tree/justfile" test) > "$log" 2>&1 9>&- || rc=$?
        if [ "$rc" -ne 0 ]; then
            if [ "$remote_ref" != "refs/heads/$default" ]; then
                echo "RED  push-check: $range: the tests fail (target/logs/push-tests.log); allowed off $default, which $target cannot merge into until they pass"
                continue
            fi
            echo "FAIL push-check: $range: the tests fail, and $default takes no red tests (full log: target/logs/push-tests.log)" >&2
            grep -E '^(test .* FAILED|failures:|error(\[|:))' "$log" | head -20 >&2 || true
            exit 1
        fi
        echo "ok   push-check ($range): tests pass"
    done
    [ "$pushed" = 1 ] || echo "ok   push-check (no commit pushed: nothing to test)"

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

# Auto-format code
fmt-fix:
    just _container fmt-fix

# Remove build artifacts
clean:
    just _container clean

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
