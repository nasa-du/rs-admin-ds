#!/bin/sh
#
# Minimal failure notification for the rs-admin systemd timers. Posts one
# generic JSON object to the webhook configured by ALERT_WEBHOOK_URL_FILE.
# Alertmanager remains the primary alerting path; this covers a backup or
# maintenance job that fails before its metrics reach Prometheus.
#
# The webhook URL is secret and is never printed.

set -eu

unit="${1:-unknown}"
url_file="${ALERT_WEBHOOK_URL_FILE:-/etc/rs-admin/secrets/alert_webhook_url}"

if [ ! -r "$url_file" ]; then
    echo "no alert webhook configured; $unit failed" >&2
    exit 0
fi

url="$(cat "$url_file")"
if [ -z "$url" ]; then
    echo "empty alert webhook; $unit failed" >&2
    exit 0
fi

host="$(hostname)"
payload="$(printf '{"alert":"RsAdminJobFailed","unit":"%s","host":"%s","text":"rs-admin job failed: %s on %s"}' \
    "$unit" "$host" "$unit" "$host")"

exec curl --fail --silent --show-error --max-time 10 \
    -H 'Content-Type: application/json' \
    --data "$payload" \
    "$url"
