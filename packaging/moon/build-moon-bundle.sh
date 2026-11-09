#!/usr/bin/env bash

# Build a .moon bundle for a Quester release.
# https://moonpkg.github.io/cli / https://github.com/moonpkg/cli

# written by @noxygalaxy, basically the same script in every project that supports .moon bundles!

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BIN_NAME="quester"
CARGO_BIN="quester"
BINARY=""
FROM=""
VERSION=""
OUTPUT=""
MOON=""
KEEP_WORK=0

usage() {
    cat <<EOF
usage: build-moon-bundle.sh [options]

  --binary PATH   binary to bundle (default: target/release/$CARGO_BIN)
  --from PATH     take the binary from a release tarball or a bare binary
  --version V     version recorded in the manifest (default: Cargo.toml)
  --output FILE   bundle to write (default: dist/$BIN_NAME-<version>-linux-x86_64.moon)
  --moon PATH     moon binary (default: the one on PATH)
  --keep-work     keep the staging folder and print where it is
  -h, --help      this text
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --binary) BINARY="$2"; shift 2 ;;
        --from) FROM="$2"; shift 2 ;;
        --version) VERSION="$2"; shift 2 ;;
        --output) OUTPUT="$2"; shift 2 ;;
        --moon) MOON="$2"; shift 2 ;;
        --keep-work) KEEP_WORK=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

# version
if [ -z "$VERSION" ]; then
    VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$REPO_ROOT/Cargo.toml" | head -1)"
fi
[ -n "$VERSION" ] || die "cannot tell the version, pass --version"

# binary
WORK="$(mktemp -d)"
cleanup() {
    if [ "$KEEP_WORK" -eq 1 ]; then
        echo "staging folder kept at $WORK"
    else
        rm -rf "$WORK"
    fi
}
trap cleanup EXIT

if [ -n "$FROM" ]; then
    [ -f "$FROM" ] || die "--from: no such file: $FROM"
    FROM_DIR="$WORK/from"
    mkdir -p "$FROM_DIR"
    if tar -xf "$FROM" -C "$FROM_DIR" 2>/dev/null; then
        FOUND="$(find "$FROM_DIR" -type f -name "$BIN_NAME" -perm -u+x | head -1)"
        [ -n "$FOUND" ] || die "--from: no '$BIN_NAME' executable inside $FROM"
        BINARY="$FOUND"
    elif [ -x "$FROM" ]; then
        BINARY="$FROM"
    else
        die "--from: cannot unpack $FROM (expected a release tarball or a bare binary)"
    fi
fi

if [ -z "$BINARY" ]; then
    BINARY="$REPO_ROOT/target/release/$CARGO_BIN"
fi
[ -f "$BINARY" ] || die "binary not found at $BINARY (build with 'cargo build --release' first, or pass --from)"

STAGE="$WORK/$BIN_NAME-moon"
echo "-= staging bundle =-"
install -Dm755 "$BINARY" "$STAGE/app/bin/$BIN_NAME"
install -Dm644 "$REPO_ROOT/packaging/$BIN_NAME.desktop" "$STAGE/desktop/$BIN_NAME.desktop"
install -Dm644 "$REPO_ROOT/assets/logo.png" "$STAGE/icon/$BIN_NAME.png"
install -Dm644 "$REPO_ROOT/README.md" "$STAGE/app/share/doc/$BIN_NAME/README.md"
install -Dm644 "$REPO_ROOT/LICENSE" "$STAGE/app/share/licenses/$BIN_NAME/LICENSE"

cat > "$STAGE/$BIN_NAME.manifest" <<EOF
dir=app
main=app/bin/$BIN_NAME
version=$VERSION
desktop=desktop/$BIN_NAME.desktop
link=$BIN_NAME
to=app/bin/$BIN_NAME
cmd=$BIN_NAME
EOF

find "$STAGE" -mindepth 1 | sed "s|^$STAGE|  |" | sort

# packing
OUT="${OUTPUT:-$REPO_ROOT/dist/$BIN_NAME-$VERSION-linux-x86_64.moon}"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"

if [ -n "$MOON" ]; then
    [ -x "$MOON" ] || die "--moon: not executable: $MOON"
elif command -v moon >/dev/null 2>&1; then
    MOON="$(command -v moon)"
fi

echo "-= packing bundle =-"
if [ -n "$MOON" ]; then
    "$MOON" bundle "$STAGE" "$OUT" --force
else
    echo "note: moon not found, packing with tar (same file: a .moon is a tar.gz)"
    tar -C "$STAGE" -czf "$OUT" .
fi
[ -f "$OUT" ] || die "no bundle was written to $OUT"

# verifying
LISTING="$(tar -tzf "$OUT")"
echo "$LISTING" | grep -q "$BIN_NAME.manifest" || die "the bundle has no $BIN_NAME.manifest"
echo "$LISTING" | grep -q "app/bin/$BIN_NAME" || die "the bundle has no app/bin/$BIN_NAME"
echo "$LISTING" | grep -q "desktop/$BIN_NAME.desktop" || die "the bundle has no desktop entry"
echo "$LISTING" | grep -q "icon/$BIN_NAME.png" || die "the bundle has no icon"

echo "-= done =-"
echo "  bundle: $OUT"
echo "  size:   $(du -h "$OUT" | cut -f1)"
echo "  sha256: $(sha256sum "$OUT" | cut -d' ' -f1)"
echo "  install it with: moon install $OUT"
