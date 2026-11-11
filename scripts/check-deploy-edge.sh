#!/usr/bin/env bash
#
# Integration check for the edge: real nginx gateway and static web container
# on a private Docker network, with a stub API upstream. Verifies TLS, host
# routing, redirects, HSTS, CSP, SPA fallback, asset caching, the auth rate
# limit and the decision not to expose health/metrics/docs.
#
# Requires Docker, curl and the development secrets
# (scripts/generate-dev-secrets.sh).

set -euo pipefail

# shellcheck source=lib/deploy-check.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib/deploy-check.sh"
cd "$root"

secret_dir="${RS_ADMIN_SECRETS_DIR:-$root/deploy/secrets-dev}"
suffix="$$"
network="rs-admin-edge-check-$suffix"
web="rs-admin-edge-web-$suffix"
api="rs-admin-edge-api-$suffix"
gateway="rs-admin-edge-gateway-$suffix"
work="$(mktemp -d)"

cleanup() {
    docker rm -f "$gateway" "$api" "$web" >/dev/null 2>&1 || true
    docker network rm "$network" >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

if [[ ! -r "$secret_dir/tls_certificate" ]]; then
    echo "error: missing development secrets; run ./scripts/generate-dev-secrets.sh" >&2
    exit 1
fi

mkdir -p "$work/site/assets"
printf '<!doctype html><title>rs-admin</title><p>rs-admin-spa</p>\n' >"$work/site/index.html"
printf 'console.log("asset");\n' >"$work/site/assets/app.js"
printf 'server {\n  listen 8080;\n  location / { add_header X-Upstream dummy always; return 200 "upstream"; }\n}\n' >"$work/api.conf"

docker network create "$network" >/dev/null

docker run --detach --rm --name "$web" --network "$network" \
    --network-alias web \
    --read-only \
    --tmpfs /tmp:mode=1777 \
    --tmpfs /etc/nginx/conf.d:uid=101,gid=101,mode=0755 \
    --volume "$root/frontend/nginx/nginx.conf:/etc/nginx/nginx.conf:ro" \
    --volume "$root/frontend/nginx/default.conf.template:/etc/nginx/templates/default.conf.template:ro" \
    --volume "$work/site:/usr/share/nginx/html:ro" \
    --env API_ORIGIN=https://api.example.com \
    --env 'NGINX_ENVSUBST_FILTER=^(API_ORIGIN)$' \
    nginxinc/nginx-unprivileged:1.29.8-alpine >/dev/null

docker run --detach --rm --name "$api" --network "$network" \
    --network-alias api \
    --volume "$work/api.conf:/etc/nginx/conf.d/default.conf:ro" \
    nginxinc/nginx-unprivileged:1.29.8-alpine >/dev/null

docker run --detach --rm --name "$gateway" --network "$network" \
    --read-only \
    --tmpfs /tmp:mode=1777 \
    --tmpfs /etc/nginx/conf.d:uid=101,gid=101,mode=0755 \
    --publish 127.0.0.1::8080 \
    --publish 127.0.0.1::8443 \
    --volume "$root/deploy/gateway/nginx.conf:/etc/nginx/nginx.conf:ro" \
    --volume "$root/deploy/gateway/templates:/etc/nginx/templates:ro" \
    --volume "$root/deploy/gateway/snippets:/etc/nginx/snippets:ro" \
    --volume "$secret_dir:/run/secrets:ro" \
    --env ADMIN_HOST=admin.example.com \
    --env API_HOST=api.example.com \
    --env AUTH_RATE_LIMIT=60r/m \
    --env AUTH_RATE_BURST=10 \
    --env 'NGINX_ENVSUBST_FILTER=^(ADMIN_HOST|API_HOST|AUTH_RATE_LIMIT|AUTH_RATE_BURST)$' \
    nginxinc/nginx-unprivileged:1.29.8-alpine >/dev/null

http_port="$(docker port "$gateway" 8080/tcp | head -n 1 | sed 's/.*://')"
https_port="$(docker port "$gateway" 8443/tcp | head -n 1 | sed 's/.*://')"
if [[ -z "$http_port" || -z "$https_port" ]]; then
    echo "error: could not determine gateway ports" >&2
    docker logs "$gateway" >&2 || true
    exit 1
fi

# Wait for the gateway and both upstreams to accept connections.
for _ in $(seq 1 100); do
    web_ready=0
    api_ready=0
    docker exec "$web" wget -q -O /dev/null http://127.0.0.1:8080/ \
        >/dev/null 2>&1 && web_ready=1
    docker exec "$api" wget -q -O /dev/null http://127.0.0.1:8080/ \
        >/dev/null 2>&1 && api_ready=1
    if [[ "$web_ready" -eq 1 && "$api_ready" -eq 1 ]] \
        && curl --noproxy '*' --silent --output /dev/null "http://127.0.0.1:${http_port}/"; then
        break
    fi
    sleep 0.2
done

for name in "$gateway" "$web"; do
    if docker top "$name" -o pid,user,comm \
        | awk 'NR > 1 { print $2 }' \
        | grep -qxE '(0|root)'; then
        fail "$name runs as a non-root user"
    else
        pass "$name runs as a non-root user"
    fi
done

if docker exec "$gateway" wget -q -O /dev/null \
    http://127.0.0.1:8080/.well-known/acme-challenge/healthz; then
    pass "gateway healthcheck command succeeds"
else
    fail "gateway healthcheck command succeeds"
fi

if docker exec "$web" wget -q -O /dev/null http://127.0.0.1:8080/; then
    pass "web healthcheck command succeeds"
else
    fail "web healthcheck command succeeds"
fi

curl_https() {
    curl --noproxy '*' --silent --show-error --resolve "$1:${https_port}:127.0.0.1" \
        --cacert "$secret_dir/internal_ca.pem" "https://$1:${https_port}$2"
}

status() {
    curl --noproxy '*' --silent --output /dev/null --write-out '%{http_code}' \
        --resolve "$1:${https_port}:127.0.0.1" \
        --cacert "$secret_dir/internal_ca.pem" "https://$1:${https_port}$2"
}

headers() {
    curl --noproxy '*' --silent --dump-header - --output /dev/null \
        --resolve "$1:${https_port}:127.0.0.1" \
        --cacert "$secret_dir/internal_ca.pem" "https://$1:${https_port}$2"
}

# --- Redirects and ACME -----------------------------------------------------
redirect="$(curl --noproxy '*' --silent --dump-header - --output /dev/null \
    --header 'Host: admin.example.com' "http://127.0.0.1:${http_port}/client/route")"
if grep -qi '^HTTP/1.1 301' <<<"$redirect" \
    && grep -qi '^Location: https://admin.example.com/client/route' <<<"$redirect"; then
    pass "HTTP requests redirect to HTTPS with 301"
else
    fail "HTTP requests redirect to HTTPS with 301"
fi

acme_status="$(curl --noproxy '*' --silent --output /dev/null --write-out '%{http_code}' \
    --header 'Host: admin.example.com' \
    "http://127.0.0.1:${http_port}/.well-known/acme-challenge/healthz")"
if [[ "$acme_status" == "204" ]]; then
    pass "ACME challenge path is exempt from the HTTPS redirect"
else
    fail "ACME challenge path is exempt from the HTTPS redirect ($acme_status)"
fi

# --- Admin SPA --------------------------------------------------------------
admin_headers="$(headers admin.example.com /)"
admin_body="$(curl_https admin.example.com /)"
if [[ "$(status admin.example.com /)" == "200" ]] && grep -q 'rs-admin-spa' <<<"$admin_body"; then
    pass "admin host serves the SPA"
else
    fail "admin host serves the SPA"
fi

if grep -qi '^Strict-Transport-Security: max-age=31536000; includeSubDomains' <<<"$admin_headers"; then
    pass "admin responses carry HSTS"
else
    fail "admin responses carry HSTS"
fi

if grep -qi '^Content-Security-Policy: .*script-src .self.;' <<<"$admin_headers" \
    && grep -qi "connect-src 'self' https://api.example.com" <<<"$admin_headers"; then
    pass "admin responses carry the strict CSP with the API origin"
else
    fail "admin responses carry the strict CSP with the API origin"
fi

if grep -qi '^Cache-Control: no-cache' <<<"$admin_headers"; then
    pass "index.html is not cached"
else
    fail "index.html is not cached"
fi

asset_headers="$(headers admin.example.com /assets/app.js)"
if grep -qi '^Cache-Control: public, max-age=31536000, immutable' <<<"$asset_headers"; then
    pass "hashed assets are immutable for a year"
else
    fail "hashed assets are immutable for a year"
fi

if [[ "$(status admin.example.com /client/side/route)" == "200" ]]; then
    pass "unknown client routes fall back to index.html"
else
    fail "unknown client routes fall back to index.html"
fi

# --- API routing and blocked operational endpoints -------------------------
api_headers="$(headers api.example.com /api/v1/users)"
if [[ "$(status api.example.com /api/v1/users)" == "200" ]] \
    && grep -qi '^X-Upstream: dummy' <<<"$api_headers"; then
    pass "API host proxies /api/v1 to the API upstream"
else
    fail "API host proxies /api/v1 to the API upstream"
fi

for blocked in /health/ready /metrics /api-docs /swagger-ui /openapi.json; do
    if [[ "$(status api.example.com "$blocked")" == "404" ]]; then
        pass "API host hides $blocked"
    else
        fail "API host hides $blocked"
    fi
done

# --- Auth rate limit --------------------------------------------------------
rate_limited=0
retry_after=0
for _ in $(seq 1 120); do
    response="$(curl --noproxy '*' --silent --dump-header - --output /dev/null \
        --resolve "api.example.com:${https_port}:127.0.0.1" \
        --cacert "$secret_dir/internal_ca.pem" \
        "https://api.example.com:${https_port}/api/v1/auth/login")"
    if grep -qiE '^HTTP/[0-9.]+ 429' <<<"$response"; then
        rate_limited=$((rate_limited + 1))
        if grep -qi '^Retry-After: 60' <<<"$response"; then
            retry_after=$((retry_after + 1))
        fi
    fi
done

if [[ "$rate_limited" -gt 0 ]]; then
    pass "auth endpoints are rate limited at the edge"
else
    fail "auth endpoints are rate limited at the edge"
fi

if [[ "$retry_after" -gt 0 ]]; then
    pass "rate-limited responses carry Retry-After"
else
    fail "rate-limited responses carry Retry-After"
fi

limited="$(curl --noproxy '*' --silent --dump-header - \
    --resolve "api.example.com:${https_port}:127.0.0.1" \
    --cacert "$secret_dir/internal_ca.pem" \
    "https://api.example.com:${https_port}/api/v1/auth/login")"
if grep -qi '^Content-Type: application/problem+json' <<<"$limited" \
    && grep -q '"code":"rate_limited"' <<<"$limited" \
    && grep -qi '^Strict-Transport-Security:' <<<"$limited"; then
    pass "rate-limited responses use the problem+json contract"
else
    fail "rate-limited responses use the problem+json contract"
fi

# --- Internal certificate probe --------------------------------------------
# blackbox_exporter reaches this SNI over the edge network to read the
# certificate's notAfter; it does not verify the chain because the probe only
# inspects expiry.
probe_status="$(curl --noproxy '*' --silent --insecure --output /dev/null \
    --write-out '%{http_code}' \
    --resolve "gateway:${https_port}:127.0.0.1" \
    "https://gateway:${https_port}/")"
if [[ "$probe_status" == "204" ]]; then
    pass "internal TLS probe serves the gateway certificate"
else
    fail "internal TLS probe serves the gateway certificate ($probe_status)"
fi

# --- Unknown SNI ------------------------------------------------------------
if curl --noproxy '*' --silent --output /dev/null \
    --resolve "unknown.example.com:${https_port}:127.0.0.1" \
    --cacert "$secret_dir/internal_ca.pem" \
    "https://unknown.example.com:${https_port}/" 2>/dev/null; then
    fail "unknown SNI is rejected"
else
    pass "unknown SNI is rejected"
fi

if [[ "$failures" -ne 0 ]]; then
    printf '\n%d edge deployment check(s) failed\n' "$failures" >&2
    for name in "$gateway" "$web" "$api"; do
        echo "--- $name logs" >&2
        docker logs "$name" 2>&1 | tail -20 >&2 || true
    done
    exit 1
fi

printf '\nall edge deployment checks passed\n'
