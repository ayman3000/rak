#!/bin/sh
# Installs rak from its GitHub releases, on Linux or macOS:
#
#   curl -fsSL https://raw.githubusercontent.com/ayman3000/rak/main/install.sh | sh
#
# Settings (all optional):
#   RAK_VERSION=v0.1.0-alpha.2   a release; default the newest, pre-releases included
#   RAK_INSTALL_DIR=~/bin        where rak goes; default ~/.local/bin
#
# It downloads the archive for this system, checks it against its published
# SHA-256 checksum, and puts the rak binary in the install folder. Nothing else
# on the system is changed.
set -eu

REPO="${RAK_REPO:-ayman3000/rak}"
INSTALL_DIR="${RAK_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${RAK_VERSION:-}"

say() { printf '%s\n' "$*"; }
fail() { printf 'rak install: %s\n' "$*" >&2; exit 1; }

# Downloads $1 to the file $2.
fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 2 "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        fail "needs curl or wget"
    fi
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "needs sha256sum or shasum to check the download"
    fi
}

case "$(uname -s)" in
    Linux) os=unknown-linux-musl ;;
    Darwin) os=apple-darwin ;;
    *) fail "no rak build for $(uname -s) yet. With Rust installed: cargo install --git https://github.com/$REPO" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) fail "no rak build for $(uname -m) yet. With Rust installed: cargo install --git https://github.com/$REPO" ;;
esac
target="$arch-$os"

tmp=$(mktemp -d)
trap 'rm -rf "${tmp:?}"' EXIT INT TERM

if [ -z "$VERSION" ]; then
    # The newest release, pre-releases included (rak is in alpha).
    fetch "https://api.github.com/repos/$REPO/releases?per_page=1" "$tmp/releases.json" \
        || fail "could not ask GitHub for rak's releases"
    VERSION=$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$tmp/releases.json" | head -n 1)
    [ -n "$VERSION" ] || fail "found no rak release on GitHub"
fi

archive="rak-$VERSION-$target.tar.gz"
base="${RAK_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$VERSION}"
say "Installing rak $VERSION for $target"
fetch "$base/$archive" "$tmp/$archive" || fail "could not download $base/$archive"
fetch "$base/$archive.sha256" "$tmp/$archive.sha256" || fail "could not download the checksum for $archive"

expected=$(cut -d ' ' -f 1 "$tmp/$archive.sha256")
actual=$(sha256 "$tmp/$archive")
[ "$expected" = "$actual" ] || fail "the download does not match its checksum; nothing was installed"

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$INSTALL_DIR"
cp "$tmp/rak-$VERSION-$target/rak" "$INSTALL_DIR/rak.new"
chmod 755 "$INSTALL_DIR/rak.new"
mv -f "$INSTALL_DIR/rak.new" "$INSTALL_DIR/rak"

say "Installed $("$INSTALL_DIR/rak" --version) to $INSTALL_DIR/rak"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) say "" && say "$INSTALL_DIR is not on your PATH. Add this line to your shell's profile:" \
        && say "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
say ""
say "Next: rak keys   (set up your API keys), then: rak -m <model>"
