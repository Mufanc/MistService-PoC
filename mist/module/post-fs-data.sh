set -eu

MODDIR=${0%/*}
TMPDIR="/data/local/tmp"

cp "$MODDIR/bin/libmist.so" "$TMPDIR"

chown shell "$TMPDIR"
chgrp shell "$TMPDIR"
chmod 771 "$TMPDIR"
chcon u:object_r:shell_data_file:s0 "$TMPDIR"

chown root "$TMPDIR/libmist.so"
chgrp root "$TMPDIR/libmist.so"
chmod 644 "$TMPDIR/libmist.so"
chcon u:object_r:system_lib_file:s0 "$TMPDIR/libmist.so"

chmod 755 "$MODDIR/bin/mist"

"$MODDIR/bin/mist" "$TMPDIR/libmist.so"
