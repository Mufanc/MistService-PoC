set -eu

MODDIR=${0%/*}
MIST_BINARY="$MODDIR/bin/mist"

chmod 744 "$MIST_BINARY"
RUST_LOG=info "$MIST_BINARY" "$MODDIR/bin/libmist.so"
