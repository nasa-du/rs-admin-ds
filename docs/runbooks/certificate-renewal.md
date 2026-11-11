# Certificate renewal and external uptime

This runbook covers the gateway TLS certificate, the internal PostgreSQL CA and
server certificate, and the external uptime provider. The `CertificateExpiry`
alert links here; the provider-owned `ExternalUptime` alert is documented in
[ExternalUptime](#externaluptime).

## CertificateExpiry

`CertificateExpiry` fires when the gateway certificate presented on the
internal probe endpoint expires in less than 14 days (warning) or 7 days
(critical), or when `blackbox_exporter` cannot read it. Follow
[Gateway TLS certificate](#1-gateway-tls-certificate) and verify with
[section 1.3](#13-verification). A probe failure with no expiry problem is a
gateway or edge-network issue, not proof of expiry.

## 1. Gateway TLS certificate

The gateway terminates TLS for `ADMIN_HOST` and `API_HOST`. Certificates are
issued by Let's Encrypt with the ACME HTTP-01 challenge served from the
`certbot-webroot` named volume. The certificate is mounted into the gateway as
`tls_certificate` / `tls_key`; a renewal must copy the new files into
`/etc/rs-admin/secrets` **in place** and reload nginx.

### 1.1 One-time host setup

```sh
sudo apt-get update
sudo apt-get install --yes certbot
sudo install -d -m 0755 /etc/letsencrypt/renewal-hooks/deploy
```

Create `/etc/letsencrypt/renewal-hooks/deploy/rs-admin.sh`:

```sh
#!/bin/sh
set -eu

secrets=/etc/rs-admin/secrets
lineage="${RENEWED_LINEAGE:?RENEWED_LINEAGE is set by certbot}"

# Write through the existing inode: Compose bind-mounts the file and a rename
# would leave the gateway reading the old certificate.
cat "$lineage/fullchain.pem" >"$secrets/tls_certificate"
cat "$lineage/privkey.pem" >"$secrets/tls_key"
chown root:101 "$secrets/tls_certificate" "$secrets/tls_key"
chmod 0440 "$secrets/tls_certificate" "$secrets/tls_key"

docker compose \
  --project-directory /opt/rs-admin/deploy \
  -f /opt/rs-admin/deploy/compose.yaml \
  exec gateway nginx -s reload
```

```sh
sudo chmod 0755 /etc/letsencrypt/renewal-hooks/deploy/rs-admin.sh
```

Issue the first certificate. The webroot is the host mountpoint of the Compose
volume; the directory is `rs-admin_certbot-webroot` for the default project
name.

```sh
webroot="$(docker volume inspect -f '{{.Mountpoint}}' rs-admin_certbot-webroot)"
sudo certbot certonly --webroot -w "$webroot" \
  --cert-name rs-admin \
  -d admin.example.com -d api.example.com \
  --deploy-hook /etc/letsencrypt/renewal-hooks/deploy/rs-admin.sh
```

`certbot.timer` runs twice daily. Verify the timer and a dry run:

```sh
systemctl list-timers certbot.timer
sudo certbot renew --dry-run --cert-name rs-admin
```

### 1.2 Renewal

Automatic renewal is the normal path; the deploy hook copies and reloads. For a
manual or forced renewal (for example after a failed timer):

```sh
sudo certbot renew --cert-name rs-admin
# Only after confirming the certificate is close to expiry or compromised:
sudo certbot renew --force-renewal --cert-name rs-admin
```

Do not edit the files in `/etc/letsencrypt/live`; certbot manages them.

### 1.3 Verification

```sh
# Both public hosts present the new certificate and the expected SANs.
for host in admin.example.com api.example.com; do
  echo "== $host =="
  openssl s_client -connect "$host:443" -servername "$host" </dev/null 2>/dev/null |
    openssl x509 -noout -subject -dates -ext subjectAltName
done

# The gateway still serves the SPA and the API edge.
curl -fsS https://admin.example.com/ >/dev/null
curl -sS -o /dev/null -w '%{http_code}\n' https://api.example.com/api/v1/auth/me
# expected 401

# Prometheus sees the new notAfter.
docker compose -f deploy/compose.observability.yaml exec prometheus \
  wget -qO- 'http://localhost:9090/api/v1/query?query=probe_ssl_earliest_cert_expiry%7Bjob%3D%22blackbox-tls%22%7D'
```

The `CertificateExpiry` alert must resolve within one evaluation interval
(15 seconds) plus the 10-minute `for` window. If the probe still fails, check
that `blackbox_exporter` can reach `https://gateway:8443/` over the edge network
and that the gateway reload succeeded (`docker compose logs gateway`).

## 2. PostgreSQL internal CA and server certificate

The API, migrate, maintenance and postgres_exporter clients verify PostgreSQL
with `sslmode=verify-full` against `internal_ca.pem`. The server certificate is
`postgres_server_cert` with its key. The `CertificateExpiry` alert does not
cover this certificate; check it during every restore drill and before the
30-day renewal point.

### 2.1 Renew the server certificate with the same CA

1. Create a new server key and CSR with the `db` SAN (and the external host
   name if the database is external):
   ```sh
   openssl req -newkey rsa:2048 -sha256 -nodes -subj "/CN=db" \
     -keyout /tmp/postgres_server_key.new -out /tmp/postgres.csr
   printf 'subjectAltName=DNS:db\nextendedKeyUsage=serverAuth\nkeyUsage=critical,digitalSignature,keyEncipherment\n' \
     >/tmp/postgres.ext
   openssl x509 -req -sha256 -days 825 -in /tmp/postgres.csr \
     -CA /etc/rs-admin/secrets/internal_ca.pem \
     -CAkey /etc/rs-admin/secrets/internal-ca.key \
     -CAcreateserial -extfile /tmp/postgres.ext \
     -out /tmp/postgres_server_cert.new
   ```
2. Install both files in place and restart the database:
   ```sh
   cat /tmp/postgres_server_cert.new >/etc/rs-admin/secrets/postgres_server_cert
   cat /tmp/postgres_server_key.new >/etc/rs-admin/secrets/postgres_server_key
   chown root:root /etc/rs-admin/secrets/postgres_server_cert \
     /etc/rs-admin/secrets/postgres_server_key
   chmod 0400 /etc/rs-admin/secrets/postgres_server_cert \
     /etc/rs-admin/secrets/postgres_server_key
   docker compose -f deploy/compose.yaml --profile local-db restart db
   ```
3. Restart the consumers so they reload the CA/server cert:
   `docker compose -f deploy/compose.yaml up -d --no-deps api`,
   `docker compose -f deploy/compose.observability.yaml restart postgres_exporter`.
   The one-shot `migrate` and `maintenance` jobs read the new files on their
   next run.

### 2.2 Rotate the internal CA

Changing the CA is disruptive because every client trusts the old CA until it
is restarted. Do it in a maintenance window and keep the old CA key offline
until the new CA is trusted everywhere.

1. Follow the first-deployment CA commands in
   [first-deploy.md](./first-deploy.md#33-internal-ca-and-postgresql-server-certificate)
   to create the new CA and a new server certificate.
2. Install the new `internal_ca.pem`, `postgres_server_cert` and
   `postgres_server_key` in place with the documented modes.
3. Restart `db`, then `api`, `postgres_exporter`, and run the one-shot
   `migrate` and `maintenance` jobs. External database clients must update
   their CA copy in the same window.
4. Keep the old CA certificate (not its private key) with the incident record
   so old logs and certificates can be explained.

### 2.3 Verification

```sh
# From a container on the data network, negotiate PostgreSQL TLS.
docker compose --profile tools run --rm --no-deps \
  --entrypoint sh api -c \
  'pg_isready -h db -p 5432 -U rs_admin -d rs_admin && \
   openssl s_client -starttls postgres -connect db:5432 -CAfile /run/secrets/internal_ca </dev/null 2>/dev/null | \
   openssl x509 -noout -subject -dates'

# The API still reads the schema and the audit chain over verified TLS.
docker compose -f deploy/compose.yaml --profile tools run --rm --no-deps \
  --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api

# The exporter still scrapes.
docker compose -f deploy/compose.observability.yaml exec postgres_exporter \
  wget -qO- http://localhost:9187/metrics | grep '^pg_up 1'
```

The API startup log must not contain a TLS verification error. If it does,
restore the previous CA/server files and restart before the database is used
for writes.

## ExternalUptime

The external uptime provider is the only detector that still works when the
host, Prometheus and Alertmanager are down. It probes the public SPA at
`https://admin.example.com/`; the API `/health/ready` endpoint is deliberately
not public (ADR-0008 2026-11-10 amendment). The provider's alert is
`ExternalUptime`, owned by the provider.

Configure:

- at least two probe regions;
- HTTPS probe of `https://admin.example.com/` expecting `200`;
- a notification channel that does not depend on this host (email, SMS or a
  chat workspace independent of the self-hosted Alertmanager path);
- a status-page or dashboard link stored in the deployment ticket.

`ExternalUptime` verification:

1. The provider dashboard shows both regions healthy after a deployment.
2. Pause the SPA container in a controlled test window and confirm the provider
   fires; restore it and confirm the recovery notification.
3. During an incident, check the provider status page before assuming the host
   is reachable. If the host is lost, follow
   [backup-restore.md](./backup-restore.md#41-actual-restore-disaster-recovery)
   and update the provider only after the SPA returns `200`.

Do not point the provider at `/health/ready`, `/metrics` or the API host; those
endpoints are not public and exposing them would violate ADR-0008 §2.6.

## Final verification

- `CertificateExpiry` is resolved and `probe_ssl_earliest_cert_expiry` shows
  more than 30 days.
- Both public hosts serve the expected certificate and a valid SPA/API edge.
- `rs-admin-verify-audit-chain` exits 0 after the database certificate change.
- The external provider reports both regions healthy and its notification path
  is independent of the host.
