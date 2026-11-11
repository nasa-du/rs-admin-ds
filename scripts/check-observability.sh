#!/usr/bin/env bash
#
# Static acceptance checks for the observability stack (ADR-0008 §6).
#
# Reads the normalized `docker compose config` output, validates the pinned
# Prometheus/Alertmanager/Loki/Promtail configuration with the same images the
# stack runs, and asserts the alert catalog, runbook links and dashboards.
#
# The development secrets must exist (scripts/generate-dev-secrets.sh).

set -euo pipefail

# shellcheck source=lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/deploy-check.sh"
cd "$root"

secret_dir="${RS_ADMIN_SECRETS_DIR:-$root/deploy/secrets-dev}"
compose_file="deploy/compose.observability.yaml"
prometheus_config="deploy/observability/prometheus/prometheus.yml"
rules_file="deploy/observability/prometheus/rules/rs-admin.yml"
alertmanager_config="deploy/observability/alertmanager/alertmanager.yml"
loki_config="deploy/observability/loki/loki-config.yaml"
promtail_config="deploy/observability/promtail/promtail-config.yaml"
runbook="docs/runbooks/alerting.md"

expect_compose() {
    local label="$1"
    local expression="$2"
    if jq -e "$expression" >/dev/null <<<"$compose"; then
        pass "$label"
    else
        fail "$label"
    fi
}

expect_file() {
    local label="$1"
    local file="$2"
    local pattern="$3"
    if [[ ! -f "$file" ]]; then
        fail "$label (missing $file)"
    elif grep -Eq -- "$pattern" "$file"; then
        pass "$label"
    else
        fail "$label ($file does not match /$pattern/)"
    fi
}

expect_file_absent() {
    local label="$1"
    local file="$2"
    local pattern="$3"
    if [[ ! -f "$file" ]]; then
        fail "$label (missing $file)"
    elif grep -Eq -- "$pattern" "$file"; then
        fail "$label ($file unexpectedly matches /$pattern/)"
    else
        pass "$label"
    fi
}

if ! command -v jq >/dev/null 2>&1; then
    echo "error: jq is required" >&2
    exit 1
fi
if ! command -v docker >/dev/null 2>&1; then
    echo "error: docker is required" >&2
    exit 1
fi
if [[ ! -r "$secret_dir/alert_webhook_url" ]]; then
    echo "error: missing development secrets; run ./scripts/generate-dev-secrets.sh" >&2
    exit 1
fi

compose="$(RS_ADMIN_SECRETS_DIR="$secret_dir" docker compose -f "$compose_file" config --format json)"

# --- Topology and hardening -------------------------------------------------
expect_compose \
    "observability stack defines every required service" \
    '.services | keys | sort == ["alert-webhook", "alertmanager", "blackbox_exporter", "grafana", "loki", "node_exporter", "postgres_exporter", "prometheus", "promtail", "textfile-merge"]'

expect_compose \
    "only Grafana publishes a host port" \
    '[.services | to_entries[] | select(.key != "grafana") | (.value.ports // [])[]] | length == 0'

expect_compose \
    "Grafana is published on loopback only" \
    '(.services.grafana.ports | length) == 1 and .services.grafana.ports[0].host_ip == "127.0.0.1" and .services.grafana.ports[0].published == "3000"'

expect_compose \
    "all observability services are read-only, non-privileged and cap-dropped" \
    '[.services[] | .read_only == true and (.cap_drop | index("ALL") != null) and ([.security_opt[] | select(. == "no-new-privileges:true")] | length == 1)] | all'

expect_compose \
    "all observability services cap the Docker log buffer" \
    '[.services[] | .logging.options["max-size"] == "10m" and .logging.options["max-file"] == "5"] | all'

expect_compose \
    "long-running services restart unless stopped" \
    '[.services | to_entries[] | select(.key != "textfile-merge") | .value.restart] | all(. == "unless-stopped")'

expect_compose \
    "images are pinned to explicit versions" \
    '[.services[].image | test(":latest$")] | any | not'

expect_compose \
    "Prometheus uses the reviewed pinned image" \
    '.services.prometheus.image == "prom/prometheus:v3.0.1"'
expect_compose \
    "Alertmanager uses the reviewed pinned image" \
    '.services.alertmanager.image == "prom/alertmanager:v0.28.0"'
expect_compose \
    "Grafana uses the reviewed pinned image" \
    '.services.grafana.image == "grafana/grafana:11.4.0"'
expect_compose \
    "Loki and Promtail use the reviewed pinned images" \
    '.services.loki.image == "grafana/loki:3.3.2" and .services.promtail.image == "grafana/promtail:3.3.2"'

expect_compose \
    "observability joins the application edge and data networks" \
    '.networks.edge.external == true and .networks.edge.name == "rs-admin_edge" and .networks.data.external == true and .networks.data.name == "rs-admin_data"'

expect_compose \
    "Prometheus scrapes the API and exporters through the application networks" \
    '[.services.prometheus.networks | keys[]] | sort == ["data", "default", "edge"]'

expect_compose \
    "Promtail is the documented root exception for Docker log files" \
    '.services.promtail.user == "0:0"'

expect_compose \
    "the textfile merge helper is the documented root exception for its volume" \
    '.services["textfile-merge"].user == "0:0"'

expect_compose \
    "the alert adapter reads the webhook and digest settings" \
    '.services["alert-webhook"].entrypoint == ["/usr/local/bin/rs-admin-alert-webhook"] and .services["alert-webhook"].environment.ALERT_WEBHOOK_URL_FILE == "/run/secrets/alert_webhook_url" and .services["alert-webhook"].environment.ALERT_DIGEST_HOUR_UTC == "9"'

expect_compose \
    "postgres_exporter uses the least-privilege monitoring role" \
    '.services.postgres_exporter.environment.DATA_SOURCE_USER == "rs_admin_monitor" and .services.postgres_exporter.environment.DATA_SOURCE_PASS_FILE == "/run/secrets/monitor_password"'

expect_compose \
    "postgres_exporter runs as the shared app uid for secret access" \
    '.services.postgres_exporter.user == "10001:10001"'

expect_compose \
    "postgres_exporter verifies PostgreSQL TLS with the internal CA" \
    '.services.postgres_exporter.environment.DATA_SOURCE_URI | contains("sslmode=verify-full") and contains("sslrootcert=/run/secrets/internal_ca")'

expect_compose \
    "node_exporter collects the textfile metrics and host filesystems" \
    '.services.node_exporter.command | tostring | contains("--collector.textfile.directory=/var/lib/node_exporter/textfile_collector")'

expect_compose \
    "the textfile merge sidecar reads both scheduled-job directories" \
    '([.services["textfile-merge"].volumes[].target] | index("/in/backup") != null) and ([.services["textfile-merge"].volumes[].target] | index("/in/maintenance") != null) and ([.services["textfile-merge"].volumes[].target] | index("/out") != null)'

expect_compose \
    "Prometheus retains metrics for 30 days" \
    '.services.prometheus.command | tostring | contains("--storage.tsdb.retention.time=30d")'

expect_compose \
    "Grafana uses a secret-backed admin password and no anonymous access" \
    '.services.grafana.environment.GF_SECURITY_ADMIN_PASSWORD__FILE == "/run/secrets/grafana_admin_password" and .services.grafana.environment.GF_AUTH_ANONYMOUS_ENABLED == "false"'

expect_file "the API image carries the alert webhook adapter" "backend/Dockerfile" 'rs-admin-alert-webhook'
expect_file "the gateway exposes the internal certificate probe" "deploy/gateway/templates/default.conf.template" 'server_name gateway;'

# --- Prometheus -------------------------------------------------------------
expect_file "Prometheus scrapes every 15 seconds" "$prometheus_config" '^  scrape_interval: 15s$'
expect_file "Prometheus evaluates rules every 15 seconds" "$prometheus_config" '^  evaluation_interval: 15s$'
expect_file "Prometheus sends alerts to Alertmanager" "$prometheus_config" 'targets: \["alertmanager:9093"\]'
expect_file "Prometheus scrapes the API metrics listener" "$prometheus_config" 'targets: \["api:9100"\]'
expect_file "Prometheus scrapes postgres_exporter" "$prometheus_config" 'targets: \["postgres_exporter:9187"\]'
expect_file "Prometheus scrapes node_exporter" "$prometheus_config" 'targets: \["node_exporter:9100"\]'
expect_file "Prometheus probes the gateway certificate through blackbox_exporter" "$prometheus_config" 'replacement: blackbox_exporter:9115'
expect_file "Prometheus loads the rs-admin rule files" "$prometheus_config" '/etc/prometheus/rules/\*\.yml'
expect_file "Prometheus uses the gateway_tls probe module" "$prometheus_config" 'module: \[gateway_tls\]'
expect_file "the gateway TLS probe uses an https target" "$prometheus_config" 'targets: \["https://gateway:8443/"\]'
expect_file "Prometheus probes internal API readiness" "$prometheus_config" 'targets: \["http://api:8080/health/ready"\]'
expect_file "Prometheus uses the api_ready probe module" "$prometheus_config" 'module: \[api_ready\]'
expect_file "blackbox_exporter defines the gateway_tls module" "deploy/observability/blackbox/blackbox.yml" '^  gateway_tls:$'
expect_file "blackbox_exporter defines the api_ready module" "deploy/observability/blackbox/blackbox.yml" '^  api_ready:$'
expect_file "the TLS probe reads the gateway certificate without verification" "deploy/observability/blackbox/blackbox.yml" 'insecure_skip_verify: true'

# --- Alert catalog and runbook links ---------------------------------------
required_alerts=(
    ApiDown
    ApiUnready
    Api5xxHigh
    ApiLatencyHigh
    LoginLatencyHigh
    RefreshReuseDetected
    LoginLockoutSpike
    GlobalLoginCircuitOpen
    AuditWriteFailure
    DbPoolSaturated
    PgArchiveFailure
    PgDown
    BackupStale
    DiskSpace
    CertificateExpiry
    CapacityCpuHigh
    CapacityMemoryHigh
    CapacityDatabaseLarge
    CapacityLoginRateHigh
)

# Most alerts land on their same-named section of alerting.md. Backup, archive
# and certificate alerts link directly to the runbook that owns the procedure.
declare -A runbook_file=(
    [BackupStale]="docs/runbooks/backup-restore.md"
    [PgArchiveFailure]="docs/runbooks/backup-restore.md"
    [CertificateExpiry]="docs/runbooks/certificate-renewal.md"
)
declare -A runbook_anchor=(
    [BackupStale]="6-alert-response"
    [PgArchiveFailure]="6-alert-response"
    [CertificateExpiry]="certificateexpiry"
    # The four capacity triggers share one review section.
    [CapacityCpuHigh]="capacity-review"
    [CapacityMemoryHigh]="capacity-review"
    [CapacityDatabaseLarge]="capacity-review"
    [CapacityLoginRateHigh]="capacity-review"
)
declare -A runbook_heading=(
    [BackupStale]='^## 6\. Alert response$'
    [PgArchiveFailure]='^## 6\. Alert response$'
    [CertificateExpiry]='^## CertificateExpiry$'
    [CapacityCpuHigh]='^## Capacity review$'
    [CapacityMemoryHigh]='^## Capacity review$'
    [CapacityDatabaseLarge]='^## Capacity review$'
    [CapacityLoginRateHigh]='^## Capacity review$'
)
runbook_base="https://github.com/nasa-du/rs-admin-ds/blob/main/"

declare -A seen=()
rule_count=0
while IFS='|' read -r name has_severity runbook_url; do
    [[ -z "$name" ]] && continue
    rule_count=$((rule_count + 1))
    seen["$name"]=1

    if [[ "$has_severity" != "1" ]]; then
        fail "alert $name declares a critical or warning severity"
    else
        pass "alert $name declares a critical or warning severity"
    fi

    if [[ -z "$runbook_url" ]]; then
        fail "alert $name has a runbook link"
        continue
    fi

    expected_file="${runbook_file[$name]:-$runbook}"
    expected_anchor="${runbook_anchor[$name]:-$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]')}"
    expected_heading="${runbook_heading[$name]:-^## ${name}$}"
    expected_url="${runbook_base}${expected_file}#${expected_anchor}"

    if [[ "$runbook_url" != "$expected_url" ]]; then
        fail "alert $name links to $expected_url (got $runbook_url)"
    elif [[ ! -f "$expected_file" ]]; then
        fail "alert $name has a $expected_file runbook"
    elif ! grep -Eq "$expected_heading" "$expected_file"; then
        fail "alert $name has a $expected_file#$expected_anchor section"
    else
        pass "alert $name links to its own runbook section"
    fi
done < <(
    awk '
        function emit() {
            if (name != "") {
                printf "%s|%d|%s\n", name, severity, runbook
            }
        }
        /^[[:space:]]*- alert: / {
            emit()
            name = $3
            severity = 0
            runbook = ""
            next
        }
        /^[[:space:]]+severity: (critical|warning)$/ {
            severity = 1
        }
        /runbook_url:/ {
            sub(/^[[:space:]]*runbook_url:[[:space:]]*/, "", $0)
            runbook = $0
        }
        END { emit() }
    ' "$rules_file"
)

for alert in "${required_alerts[@]}"; do
    if [[ -n "${seen[$alert]:-}" ]]; then
        pass "alert catalog covers $alert"
    else
        fail "alert catalog covers $alert"
    fi
done

if [[ "$rule_count" -ge 15 ]]; then
    pass "alert catalog defines at least one rule per catalog entry"
else
    fail "alert catalog defines at least one rule per catalog entry (found $rule_count)"
fi

expect_file \
    "the alert catalog records provider-owned ExternalUptime" \
    "$rules_file" \
    'ExternalUptime is provider-owned'
if grep -Eqi '^## ExternalUptime$' "$runbook"; then
    pass "ExternalUptime has a provider-owned runbook section"
else
    fail "ExternalUptime has a provider-owned runbook section"
fi
if grep -Eqi '^## ExternalUptime$' docs/runbooks/certificate-renewal.md; then
    pass "ExternalUptime links to the certificate and uptime runbook"
else
    fail "ExternalUptime links to the certificate and uptime runbook"
fi
expect_file \
    "the alert catalog points ExternalUptime at the certificate runbook" \
    "$rules_file" \
    'certificate-renewal\.md#externaluptime'

expect_file "critical alerts are marked critical" "$rules_file" 'severity: critical'
expect_file "warning alerts are marked warning" "$rules_file" 'severity: warning'
expect_file "CertificateExpiry also fires when its probe fails" "$rules_file" 'probe_success\{job="blackbox-tls"\} == 0'
expect_file_absent "alert selectors never reference identifier labels" "$rules_file" '(username|email|request_id|ip)='

# --- Alertmanager -----------------------------------------------------------
expect_file "Alertmanager routes critical alerts immediately" "$alertmanager_config" 'url: http://alert-webhook:8081/alertmanager/critical'
expect_file "Alertmanager routes warnings to the digest receiver" "$alertmanager_config" 'url: http://alert-webhook:8081/alertmanager/warning'
expect_file "Alertmanager sends resolved notifications" "$alertmanager_config" 'send_resolved: true'
expect_file "Alertmanager repeats warnings daily at most" "$alertmanager_config" 'repeat_interval: 24h'
expect_file "Alertmanager suppresses matching warnings under a critical" "$alertmanager_config" 'inhibit_rules:'

# --- Grafana ----------------------------------------------------------------
expect_file "Grafana provisions Prometheus" "deploy/observability/grafana/provisioning/datasources/datasources.yml" 'url: http://prometheus:9090'
expect_file "Grafana provisions Loki" "deploy/observability/grafana/provisioning/datasources/datasources.yml" 'url: http://loki:3100'
expect_file "Grafana loads dashboards from the provisioned directory" "deploy/observability/grafana/provisioning/dashboards/dashboards.yml" 'path: /var/lib/grafana/dashboards'

dashboard_files=(deploy/observability/grafana/dashboards/*.json)
if [[ "${#dashboard_files[@]}" -eq 3 ]]; then
    pass "Grafana ships exactly three dashboards"
else
    fail "Grafana ships exactly three dashboards (found ${#dashboard_files[@]})"
fi

dashboard_expressions=""
for dashboard in "${dashboard_files[@]}"; do
    if jq -e '.title and .uid and (.panels | length >= 5) and ([.panels[].targets[]?.expr] | length > 0)' \
        "$dashboard" >/dev/null; then
        pass "dashboard $(basename "$dashboard") is valid and has panels"
    else
        fail "dashboard $(basename "$dashboard") is valid and has panels"
    fi
    dashboard_expressions+="$(jq -r '.panels[].targets[]?.expr' "$dashboard")"$'\n'
done

for metric in \
    auth_login_attempts_total \
    auth_login_lockouts_total \
    auth_refresh_reuse_detected_total \
    auth_password_hash_seconds \
    http_requests_total \
    http_request_duration_seconds_bucket \
    db_pool_connections \
    pg_stat_database_numbackends \
    pg_stat_archiver_failed_count \
    node_filesystem_avail_bytes \
    pgbackrest_backup_last_success_timestamp \
    probe_ssl_earliest_cert_expiry; do
    if grep -Fq "$metric" <<<"$dashboard_expressions"; then
        pass "dashboards query $metric"
    else
        fail "dashboards query $metric"
    fi
done

# --- Loki and Promtail ------------------------------------------------------
expect_file "Loki retains logs for 14 days" "$loki_config" '^  retention_period: 336h$'
expect_file "Loki compactor enforces retention" "$loki_config" '^  retention_enabled: true$'
expect_file "Loki stores delete requests on the filesystem" "$loki_config" '^  delete_request_store: filesystem$'

expect_file "Promtail discovers every Docker container" "$promtail_config" 'docker_sd_configs:'
expect_file "Promtail reads the Docker socket" "$promtail_config" 'host: unix:///var/run/docker.sock'
expect_file "Promtail pushes to Loki" "$promtail_config" 'url: http://loki:3100/loki/api/v1/push'
expect_file "Promtail redacts authorization headers" "$promtail_config" 'authorization\|cookie'
expect_file "Promtail redacts password and token fields" "$promtail_config" 'password\|passwd\|secret\|token'
expect_file "Promtail redacts bearer tokens" "$promtail_config" 'Bearer \[REDACTED\]'
expect_file "Promtail redacts JWT-shaped values" "$promtail_config" 'REDACTED_JWT'
expect_file_absent "Promtail never uses request_id as a label" "$promtail_config" 'target_label: request_id'

# --- External uptime --------------------------------------------------------
expect_file "external uptime documentation names the public SPA" "deploy/observability/uptime/README.md" 'admin\.example\.com/'
expect_file "external uptime documentation records the readiness decision" "deploy/observability/uptime/README.md" '/health/ready'
expect_file "external uptime documentation names the provider-owned alert" "deploy/observability/uptime/README.md" 'ExternalUptime'

# --- Validate the real configuration files with the pinned images ----------
prometheus_image="$(jq -r '.services.prometheus.image' <<<"$compose")"
alertmanager_image="$(jq -r '.services.alertmanager.image' <<<"$compose")"
loki_image="$(jq -r '.services.loki.image' <<<"$compose")"
promtail_image="$(jq -r '.services.promtail.image' <<<"$compose")"

if docker run --rm --entrypoint /bin/promtool \
    --volume "$root/deploy/observability/prometheus:/etc/prometheus:ro" \
    "$prometheus_image" check config /etc/prometheus/prometheus.yml >/dev/null 2>&1; then
    pass "promtool accepts prometheus.yml and its rule files"
else
    fail "promtool accepts prometheus.yml and its rule files"
fi

if docker run --rm --entrypoint /bin/promtool \
    --volume "$root/deploy/observability/prometheus:/etc/prometheus:ro" \
    "$prometheus_image" check rules /etc/prometheus/rules/rs-admin.yml >/dev/null 2>&1; then
    pass "promtool accepts the rs-admin alert rules"
else
    fail "promtool accepts the rs-admin alert rules"
fi

if docker run --rm --entrypoint /bin/amtool \
    --volume "$root/deploy/observability/alertmanager:/etc/alertmanager:ro" \
    "$alertmanager_image" check-config /etc/alertmanager/alertmanager.yml >/dev/null 2>&1; then
    pass "amtool accepts alertmanager.yml"
else
    fail "amtool accepts alertmanager.yml"
fi

work_root="$(mktemp -d)"
trap 'rm -rf "$work_root"' EXIT
loki_work="$work_root/loki"
promtail_work="$work_root/promtail"
mkdir -p "$loki_work" "$promtail_work"

if docker run --rm \
    --volume "$root/$loki_config:/etc/loki/loki-config.yaml:ro" \
    --volume "$loki_work:/loki" \
    "$loki_image" -config.file=/etc/loki/loki-config.yaml -verify-config >/dev/null 2>&1; then
    pass "Loki accepts loki-config.yaml"
else
    fail "Loki accepts loki-config.yaml"
fi

if docker run --rm \
    --volume "$root/$promtail_config:/etc/promtail/promtail-config.yaml:ro" \
    --volume "$promtail_work:/var/lib/promtail" \
    "$promtail_image" -config.file=/etc/promtail/promtail-config.yaml -check-syntax >/dev/null 2>&1; then
    pass "Promtail accepts promtail-config.yaml"
else
    fail "Promtail accepts promtail-config.yaml"
fi

if [[ "$failures" -ne 0 ]]; then
    printf '\n%d observability check(s) failed\n' "$failures" >&2
    exit 1
fi

printf '\nall observability checks passed\n'
