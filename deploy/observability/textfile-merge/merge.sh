#!/bin/sh
#
# node_exporter has one textfile collector directory, while the backup job and
# the maintenance job write into different host directories (they run as
# different container users). This tiny sidecar copies both Prometheus textfile
# outputs into the shared read-only volume that node_exporter collects.
#
# The source writers replace their files atomically, so a copy is always a
# complete snapshot. A missing directory or file is normal before the first
# scheduled run.

set -eu

mkdir -p /out

while true; do
    for directory in /in/backup /in/maintenance; do
        if [ -d "$directory" ]; then
            cp -f "$directory"/*.prom /out/ 2>/dev/null || true
        fi
    done
    sleep 15
done
