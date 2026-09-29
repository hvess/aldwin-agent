#!/usr/bin/env bash
#
# The release recipe. One file, three modes, no step that exists only inside
# a CI runner's YAML.
#
# **Why a script rather than workflow steps.** A recipe spread across `run:`
# blocks can only be executed by GitHub, which means it cannot be run before
# it is pushed, cannot be tested, and cannot be reproduced by anyone checking
# that a published binary matches the source. Two bugs in the previous
# version were of exactly that kind — a missing checkout, and a guard whose
# `[ cond ] && action` exited the step under `bash -e` — and neither was
# findable without pushing a tag.
#
# **Why shell rather than a Rust subcommand**, against this project's own
# precedent in `aldwin-review`: a third party verifying a release has to be
# able to read and run the recipe *without building anything first*. A Rust
# tool would have to be compiled before it could check a compilation.
#
# **What is deterministic, and what is not.**
#
# Deterministic, and enforced here: the compiler (rust-toolchain.toml), the
# dependency graph (`--locked`), the absolute paths baked into the binary
# (`--remap-path-prefix`, without which the shipped binary embedded the
# builder's home directory 493 times), and every byte of the archive.
#
# Not achievable here, and stated rather than implied: macOS binaries must be
# built on macOS, because cross-compiling Darwin needs Apple's SDK and its
# licence restricts that to Apple hardware. So the Linux artifacts are
# reproducible by anyone with the pinned toolchain, and the macOS artifact is
# reproducible by anyone with the pinned toolchain *and* a Mac. That is a
# property of Apple's licensing, not of this script.
set -euo pipefail

usage() {
    cat >&2 <<'USAGE'
usage:
  release.sh check <tag>        verify the tag matches the workspace version
  release.sh build <target>     build one target into dist/bin/<target>/aldwin
  release.sh package            archive every built target, deterministically
  release.sh sign <key-file>    sign dist/SHA256SUMS with an SSH private key
  release.sh verify [signers]   verify that signature (default: ./allowed_signers)

  Build on the OS that matches the target; package only ever on Linux.
USAGE
    exit 2
}

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

workspace_version() {
    # First `version = "..."` under [workspace.package]. `grep -m1` over the
    # whole file would find a dependency's version if the layout ever moves.
    # Split on the quotes rather than substituting them away: a greedy
    # `gsub(/.*"|".*/, "")` eats the value too, and silently yields an empty
    # string that compares unequal to every tag.
    awk '/^\[workspace\.package\]/ { in_section = 1; next }
         /^\[/                     { in_section = 0 }
         in_section && /^version/  { split($0, a, "\""); print a[2]; exit }' Cargo.toml
}

# ── check ────────────────────────────────────────────────────────────────
#
# The binary reports Cargo.toml's version in `--version` and in the TUI's top
# bar. Releases were once tag-only, so the manifest sat at 0.1.0 through
# eleven tagged releases and every build claimed to be 0.1.0.
cmd_check() {
    local tag="${1:-}" manifest
    [ -n "$tag" ] || usage
    manifest="$(workspace_version)"
    tag="${tag#v}"
    if [ "$manifest" != "$tag" ]; then
        echo "error: tag v$tag does not match the workspace version $manifest" >&2
        echo "       bump Cargo.toml, commit, then tag the same number" >&2
        exit 1
    fi
    echo "version $manifest matches tag v$tag"
}

# ── build ────────────────────────────────────────────────────────────────
cmd_build() {
    local target="${1:-}"
    [ -n "$target" ] || usage

    # `rustup` is what makes rust-toolchain.toml mean anything: it reads the
    # channel from that file and installs the target on demand. Without it —
    # a distro toolchain, say — the pin is inert, so the build may still be
    # perfectly good and simply will not match a published hash. Say that,
    # loudly, rather than let the script imply a guarantee it is not
    # providing on this machine.
    if command -v rustup >/dev/null 2>&1; then
        rustup target add "$target" >/dev/null
    else
        # The version *number* is not the compiler. A distro rebuild reports
        # the pinned number and is a different binary: Arch's `rust
        # 1:1.98.1-1` says `1.98.1` against a pin of `1.98.1`, and the
        # archive it produced differed from the published one — while this
        # guard, comparing those two strings, stayed quiet and printed
        # "pinned 1.98.1, building with 1.98.1" as though that settled it.
        # A channel name carries no commit hash to compare against, so
        # without rustup this cannot be *verified* either way; say that
        # instead of implying agreement, and print the commit hash so it can
        # be compared against the one a release was built with by hand.
        local pinned actual commit
        pinned="$(awk -F'"' '/^channel/ { print $2; exit }' rust-toolchain.toml)"
        actual="$(rustc --version | cut -d' ' -f2)"
        # No `exit` in this awk, unlike the two that read files above: it
        # would close the pipe while `rustc -vV` was still writing, and
        # `pipefail` turns that SIGPIPE into a failed build. There is one
        # commit-hash line, so reading to the end costs nothing.
        commit="$(rustc -vV | awk '/^commit-hash/ { print $2 }')"
        echo "warning: rustup is not installed, so rust-toolchain.toml is not in effect." >&2
        echo "         pinned $pinned, building with $actual (commit ${commit:-unknown})." >&2
        if [ "$pinned" != "$actual" ]; then
            echo "         these differ — this build will NOT reproduce the published hashes." >&2
        else
            echo "         a matching number is not a matching compiler, so this build may still" >&2
            echo "         not reproduce them. Install rustup to build through the pin." >&2
        fi
    fi

    # Absolute paths are the single biggest obstacle to a reproducible Rust
    # binary: panic messages and debug info embed the full path of every
    # source file, so `$HOME` and `$CARGO_HOME` end up inside the artifact and
    # two machines can never agree. Remapping both to fixed roots removes the
    # machine from the output — and stops shipping the builder's username to
    # everyone who downloads it.
    local cargo_home="${CARGO_HOME:-$HOME/.cargo}"
    export RUSTFLAGS="--remap-path-prefix=${cargo_home}=/cargo --remap-path-prefix=${root}=/src${RUSTFLAGS:+ $RUSTFLAGS}"

    cargo build --release --locked --target "$target" -p aldwin-cli

    mkdir -p "dist/bin/$target"
    cp "target/$target/release/aldwin" "dist/bin/$target/aldwin"
    echo "built $target"
}

# ── package ──────────────────────────────────────────────────────────────
#
# **Linux only, deliberately.** macOS ships bsdtar, which rejects `--sort`,
# `--owner` and `--mtime`; packaging there would need a second recipe with
# different flags, which is the deviance this script exists to remove. The
# Mac runner builds a binary and nothing else.
cmd_package() {
    # The version is *read*, never passed in. Taking it as an argument meant
    # two sources of truth for one fact, and the caller with the weaker claim
    # would have won: on a `workflow_dispatch` run there is no tag, so
    # `github.ref_name` is a branch name and every archive would have been
    # called `aldwin-vmain-...`. `check` already guarantees the tag and the
    # manifest agree, so the manifest is the only thing worth reading.
    local version
    version="$(workspace_version)"
    [ -n "$version" ] || { echo "error: could not read the workspace version" >&2; exit 1; }

    if ! tar --version 2>/dev/null | head -1 | grep -q 'GNU tar'; then
        echo "error: packaging needs GNU tar (macOS bsdtar cannot write a deterministic archive)" >&2
        exit 1
    fi
    [ -d dist/bin ] || { echo "error: no dist/bin — run 'build' first" >&2; exit 1; }

    # The signature too: a stale one beside fresh checksums fails `verify`,
    # and makes `sign` stop to ask about overwriting it.
    rm -f dist/*.tar.gz dist/SHA256SUMS dist/SHA256SUMS.sig
    local target
    for target in $(ls dist/bin | sort); do
        local bin="dist/bin/$target/aldwin"
        [ -f "$bin" ] || { echo "error: $bin is missing" >&2; exit 1; }

        # Normalise everything tar would otherwise copy from the filesystem.
        # The mode matters because `upload-artifact` does not preserve it, so
        # a binary that arrived here from another job may have lost +x.
        chmod 755 "$bin"
        touch -d "@0" "$bin"

        # `--sort=name` fixes member order, `--mtime/--owner/--group` fix the
        # metadata, and `gzip -n` omits the timestamp gzip would otherwise
        # write into its own header. Without these an identical binary built
        # an hour later produces a different archive hash — measured, not
        # assumed.
        tar --sort=name --mtime="@0" --owner=0 --group=0 --numeric-owner \
            -cf - -C "dist/bin/$target" aldwin \
            | gzip -n > "dist/aldwin-v${version}-${target}.tar.gz"
        echo "packaged $target"
    done

    ( cd dist && sha256sum ./*.tar.gz | sed 's| \./| |' > SHA256SUMS )
    cat dist/SHA256SUMS
}

# ── sign / verify ────────────────────────────────────────────────────────
#
# `ssh-keygen -Y`, not cosign.
#
# cosign was the first choice and does not work for this. On 3.x it has
# deprecated detached signatures and refuses offline key signing outright:
# `sign-blob --output-signature` errors with "must specify --bundle with
# --new-bundle-format", and the bundle path errors with "--tlog-upload=false
# is not supported with --signing-config". Getting a signature that is not
# published to Sigstore's public log now means hand-writing a signing-config
# with the log services stripped out — more moving parts, for a worse result.
#
# `ssh-keygen -Y` is already installed everywhere, needs no network, no log
# and no service, and its file format has been stable for years. The
# signature covers SHA256SUMS, and SHA256SUMS covers every archive, so one
# signature is the whole chain.
#
# The namespace is what stops a signature made for one purpose being replayed
# as another; ssh-keygen requires it on both sides and refuses a mismatch.
#
# Which releases these constants verify is `allowed_signers`' to say.
# `/update` checks against the same two: `crates/cli/src/update.rs`.
readonly SIG_NAMESPACE="aldwin-release"
readonly SIG_PRINCIPAL="release@aldwin"

cmd_sign() {
    local key="${1:-}"
    [ -n "$key" ] || usage
    [ -f "$key" ] || { echo "error: no such key file: $key" >&2; exit 1; }
    [ -f dist/SHA256SUMS ] || { echo "error: no dist/SHA256SUMS — run 'package' first" >&2; exit 1; }

    ssh-keygen -Y sign -f "$key" -n "$SIG_NAMESPACE" dist/SHA256SUMS
}

cmd_verify() {
    local signers="${1:-allowed_signers}"
    [ -f "$signers" ] || { echo "error: no signers file: $signers" >&2; exit 1; }
    [ -f dist/SHA256SUMS.sig ] || { echo "error: no signature — run 'sign' first" >&2; exit 1; }

    ssh-keygen -Y verify -f "$signers" -I "$SIG_PRINCIPAL" \
        -n "$SIG_NAMESPACE" -s dist/SHA256SUMS.sig < dist/SHA256SUMS
}

case "${1:-}" in
    check)   shift; cmd_check   "$@" ;;
    build)   shift; cmd_build   "$@" ;;
    package) shift; cmd_package "$@" ;;
    sign)    shift; cmd_sign    "$@" ;;
    verify)  shift; cmd_verify  "$@" ;;
    *)       usage ;;
esac
