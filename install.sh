#!/bin/sh
# Installs the latest Aldwin release:
#
#   curl -fsSL https://raw.githubusercontent.com/hvess/aldwin-agent/main/install.sh | sh
#
# It downloads the archive for this machine, checks it against the release's
# SHA256SUMS, and checks SHA256SUMS against its signature using the
# `allowed_signers` committed at the release's tag — not one served next to
# the signature. Nothing is installed unless both checks pass.
#
# ALDWIN_VERSION      a tag such as v0.4.0 (default: the latest release)
# ALDWIN_INSTALL_DIR  where `aldwin` goes (default: ~/.local/bin)
# ALDWIN_REPO         the GitHub repository releases come from, for a fork
set -eu

repo="${ALDWIN_REPO:-hvess/aldwin-agent}"
install_dir="${ALDWIN_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
    echo "aldwin install: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is needed and isn't installed."
}

need curl
need tar
need ssh-keygen

case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
    Darwin/arm64) target=aarch64-apple-darwin ;;
    *) fail "there's no release for $(uname -s) on $(uname -m) yet. Linux on x86_64 and Apple Silicon Macs are supported; anything else can build from source." ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$@"; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$@"; }
else
    fail "sha256sum or shasum is needed and neither is installed."
fi

# The latest release's tag, read from where GitHub redirects /latest, so no
# API token or rate limit is involved.
version="${ALDWIN_VERSION:-}"
if [ -z "$version" ]; then
    latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") \
        || fail "couldn't reach GitHub to find the latest release."
    version="${latest##*/}"
    case "$version" in
        v[0-9]*) ;;
        *) fail "couldn't tell which release is the latest." ;;
    esac
fi

archive="aldwin-$version-$target.tar.gz"
base="https://github.com/$repo/releases/download/$version"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
cd "$tmp"

echo "Downloading Aldwin $version for $target"
for file in "$archive" SHA256SUMS SHA256SUMS.sig; do
    curl -fsSL -o "$file" "$base/$file" || fail "couldn't download $file from release $version."
done
curl -fsSL -o allowed_signers "https://raw.githubusercontent.com/$repo/$version/allowed_signers" \
    || fail "couldn't download allowed_signers at $version."

# Releases before the rename were signed as mjolnir, with the same key.
verified=
for name in aldwin mjolnir; do
    if ssh-keygen -Y verify -f allowed_signers -I "release@$name" -n "$name-release" \
        -s SHA256SUMS.sig < SHA256SUMS >/dev/null 2>&1; then
        verified=1
        break
    fi
done
[ -n "$verified" ] || fail "SHA256SUMS doesn't carry a valid release signature, so nothing was installed."

grep " $archive\$" SHA256SUMS > expected || fail "SHA256SUMS doesn't list $archive."
sha256 -c expected >/dev/null 2>&1 || fail "$archive doesn't match its checksum, so nothing was installed."

tar -xzf "$archive" aldwin
mkdir -p "$install_dir"
mv aldwin "$install_dir/aldwin"
chmod 755 "$install_dir/aldwin"

echo "Installed Aldwin $version to $install_dir/aldwin"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) echo "$install_dir isn't on your PATH yet. Add it, then run: aldwin" ;;
esac
