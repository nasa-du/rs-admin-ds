# First deployment

This is the first-install runbook for the single-host Compose stack. It covers
host preparation, production key material, the first release, the bootstrap
super administrator and the acceptance checks. The steady-state procedures are
[ci-cd.md](./ci-cd.md) for releases and [backup-restore.md](./backup-restore.md)
for backups.

All commands run from the repository checkout on the deployment host, assumed
to be `/opt/rs-admin`. The host timezone is UTC.

## 1. Host prerequisites

- Ubuntu 24.04 LTS or Debian 12, 4 vCPU / 8 GiB RAM / 100 GiB SSD minimum
  (ADR-0008 §1).
- Docker Engine 27+ with Compose v2, `git`, `curl`, `openssl`, `jq`, `certbot`.
- Two DNS names in the same registrable domain, for example
  `admin.example.com` and `api.example.com`, pointing at the host.
- Inbound TCP 80 and 443 open; SSH restricted to administrators. No database
  port is published.
- An S3-compatible bucket for pgBackRest and an access key scoped to it.
- A chat webhook URL (Feishu/DingTalk/WeCom/generic) for alerts, or an empty
  `alert_webhook_url` if delivery is intentionally disabled.

Check the baseline:

```sh
docker version
docker compose version
timedatectl show -p Timezone --value        # UTC
df -h /var/lib/docker
```

## 2. Check out the release

```sh
sudo install -d -m 0755 /opt/rs-admin
sudo chown "$USER": /opt/rs-admin
git clone https://github.com/nasa-du/rs-admin-ds.git /opt/rs-admin
cd /opt/rs-admin
git fetch --tags
git checkout <release-tag-or-sha>
docker login ghcr.io                        # read-only GHCR token
```

Only deploy a commit that passed the required checks on `main`. Record the
commit in the deployment ticket.

## 3. Create the production secrets

Key material lives outside the repository. The full inventory, readers and
modes are in [deploy/secrets/README.md](../../deploy/secrets/README.md). Create
the directory first:

```sh
sudo install -d -m 0711 -o root -g root /etc/rs-admin/secrets
export SECRETS_DIR=/etc/rs-admin/secrets
umask 077
```

### 3.1 Application keys

```sh
# Access-token signing key (Ed25519 PKCS#8).
openssl genpkey -algorithm ED25519 -out "$SECRETS_DIR/jwt-ed25519-private.pem"

# Refresh derivation and throttle HMAC keys (32 bytes, base64).
openssl rand -base64 32 >"$SECRETS_DIR/refresh-derivation.key"
openssl rand -base64 32 >"$SECRETS_DIR/throttle.key"

# Audit chain keyring: key id -> base64 key. The id must match
# AUDIT_CHAIN_KEY_ID in deploy/.env.
printf '{"%s":"%s"}\n' "prod-2026-11" "$(openssl rand -base64 32)" \
  >"$SECRETS_DIR/audit-chain-keys.json"
```

### 3.2 Database passwords and role DSNs

```sh
for name in postgres_admin migrator runtime cleanup backup monitor; do
  openssl rand -hex 24 >"$SECRETS_DIR/${name}_password"
done

# The DSNs are consumed by containers; the CA is mounted at
# /run/secrets/internal_ca.
printf 'postgres://rs_admin_runtime:%s@db:5432/rs_admin?sslmode=verify-full&sslrootcert=/run/secrets/internal_ca\n' \
  "$(cat "$SECRETS_DIR/runtime_password")" >"$SECRETS_DIR/database_url"
printf 'postgres://rs_admin_migrator:%s@db:5432/rs_admin?sslmode=verify-full&sslrootcert=/run/secrets/internal_ca\n' \
  "$(cat "$SECRETS_DIR/migrator_password")" >"$SECRETS_DIR/database_url_migrator"
printf 'postgres://rs_admin_cleanup:%s@db:5432/rs_admin?sslmode=verify-full&sslrootcert=/run/secrets/internal_ca\n' \
  "$(cat "$SECRETS_DIR/cleanup_password")" >"$SECRETS_DIR/database_url_cleanup"
```

### 3.3 Internal CA and PostgreSQL server certificate

The PostgreSQL server certificate must cover the Compose service name `db` and
be signed by the internal CA that the API trusts:

```sh
openssl req -x509 -newkey rsa:3072 -sha256 -nodes -days 1825 \
  -subj "/CN=rs-admin-internal-ca" \
  -keyout "$SECRETS_DIR/internal-ca.key" \
  -out "$SECRETS_DIR/internal_ca.pem"

openssl req -newkey rsa:2048 -sha256 -nodes \
  -subj "/CN=db" \
  -keyout "$SECRETS_DIR/postgres_server_key" \
  -out /tmp/postgres.csr

printf 'subjectAltName=DNS:db\nextendedKeyUsage=serverAuth\nkeyUsage=critical,digitalSignature,keyEncipherment\n' \
  >/tmp/postgres.ext
openssl x509 -req -sha256 -days 825 \
  -in /tmp/postgres.csr \
  -CA "$SECRETS_DIR/internal_ca.pem" \
  -CAkey "$SECRETS_DIR/internal-ca.key" \
  -CAcreateserial \
  -extfile /tmp/postgres.ext \
  -out "$SECRETS_DIR/postgres_server_cert"
rm -f /tmp/postgres.csr /tmp/postgres.ext
```

Keep `internal-ca.key` offline after signing; it is not mounted by Compose.

### 3.4 Gateway certificate

Obtain the public certificate with certbot and install it in place. The full
renewal procedure, including the reload and verification commands, is in
[certificate-renewal.md](./certificate-renewal.md). The first deployment uses
the same commands with `certonly`.

### 3.5 Backup repository and alert secrets

```sh
# From the S3-compatible provider, scoped to the bucket prefix.
printf '%s' '<access-key>' >"$SECRETS_DIR/backup_s3_access_key"
printf '%s' '<secret-key>' >"$SECRETS_DIR/backup_s3_secret_key"
# At least 32 bytes; store an offline copy in the password manager.
openssl rand -hex 32 >"$SECRETS_DIR/backup_repo_cipher_pass"

# Empty file means "accept alerts without delivery".
printf '%s' '<chat-webhook-url>' >"$SECRETS_DIR/alert_webhook_url"
openssl rand -hex 24 >"$SECRETS_DIR/grafana_admin_password"
```

**Losing `backup_repo_cipher_pass` makes every backup unreadable. Keep an
offline copy.**

### 3.6 File modes

Compose file secrets keep the host file mode. Set the readers from
[deploy/secrets/README.md](../../deploy/secrets/README.md):

```sh
chown root:root "$SECRETS_DIR"
chmod 0711 "$SECRETS_DIR"

# API, migrate and maintenance (uid/gid 10001).
chown root:10001 "$SECRETS_DIR"/database_url \
  "$SECRETS_DIR"/database_url_migrator "$SECRETS_DIR"/database_url_cleanup \
  "$SECRETS_DIR"/internal_ca.pem "$SECRETS_DIR"/jwt-ed25519-private.pem \
  "$SECRETS_DIR"/refresh-derivation.key "$SECRETS_DIR"/throttle.key \
  "$SECRETS_DIR"/audit-chain-keys.json "$SECRETS_DIR"/monitor_password \
  "$SECRETS_DIR"/alert_webhook_url
chmod 0440 "$SECRETS_DIR"/database_url "$SECRETS_DIR"/database_url_migrator \
  "$SECRETS_DIR"/database_url_cleanup "$SECRETS_DIR"/internal_ca.pem \
  "$SECRETS_DIR"/jwt-ed25519-private.pem \
  "$SECRETS_DIR"/refresh-derivation.key "$SECRETS_DIR"/throttle.key \
  "$SECRETS_DIR"/audit-chain-keys.json "$SECRETS_DIR"/monitor_password \
  "$SECRETS_DIR"/alert_webhook_url

# Gateway (nginx uid 101).
chown root:101 "$SECRETS_DIR"/tls_certificate "$SECRETS_DIR"/tls_key
chmod 0440 "$SECRETS_DIR"/tls_certificate "$SECRETS_DIR"/tls_key

# Backup job (postgres uid 70).
chown root:70 "$SECRETS_DIR"/backup_s3_access_key \
  "$SECRETS_DIR"/backup_s3_secret_key "$SECRETS_DIR"/backup_repo_cipher_pass
chmod 0440 "$SECRETS_DIR"/backup_s3_access_key \
  "$SECRETS_DIR"/backup_s3_secret_key "$SECRETS_DIR"/backup_repo_cipher_pass

# Database entrypoint (root) reads these before dropping to postgres.
chown root:root "$SECRETS_DIR"/postgres_admin_password \
  "$SECRETS_DIR"/migrator_password "$SECRETS_DIR"/runtime_password \
  "$SECRETS_DIR"/cleanup_password "$SECRETS_DIR"/backup_password \
  "$SECRETS_DIR"/postgres_server_cert "$SECRETS_DIR"/postgres_server_key
chmod 0400 "$SECRETS_DIR"/postgres_admin_password \
  "$SECRETS_DIR"/migrator_password "$SECRETS_DIR"/runtime_password \
  "$SECRETS_DIR"/cleanup_password "$SECRETS_DIR"/backup_password \
  "$SECRETS_DIR"/postgres_server_cert "$SECRETS_DIR"/postgres_server_key

# Grafana (uid 472).
chown root:472 "$SECRETS_DIR"/grafana_admin_password
chmod 0440 "$SECRETS_DIR"/grafana_admin_password
```

Verify that no file is world-readable:

```sh
find "$SECRETS_DIR" -maxdepth 1 -type f -perm /004 -print
# expected: no output
```

## 4. Configure `deploy/.env`

```sh
cd /opt/rs-admin
cp deploy/.env.example deploy/.env
chmod 0640 deploy/.env
```

Set at least:

```ini
RS_ADMIN_SECRETS_DIR=/etc/rs-admin/secrets
APP_ENV=production
ADMIN_HOST=admin.example.com
API_HOST=api.example.com
ADMIN_ORIGIN=https://admin.example.com
PUBLIC_API_ORIGIN=https://api.example.com
CORS_ALLOWED_ORIGINS=https://admin.example.com
VITE_API_BASE_URL=https://api.example.com
DB_IMAGE=ghcr.io/nasa-du/rs-admin-ds/db:<tag>
API_IMAGE=ghcr.io/nasa-du/rs-admin-ds/api:<tag>
WEB_IMAGE=ghcr.io/nasa-du/rs-admin-ds/web:<tag>
GIT_COMMIT=<sha>
BACKUP_S3_BUCKET=<bucket>
BACKUP_S3_ENDPOINT=<https endpoint>
BACKUP_S3_REGION=<region>
BACKUP_S3_URI_STYLE=path
AUDIT_CHAIN_KEY_ID=prod-2026-11
RESTORE_DRILL_STATE_DIR=/var/lib/rs-admin/restore-drills
UPGRADE_DRILL_STATE_DIR=/var/lib/rs-admin/upgrade-drills
```

`VITE_API_BASE_URL` is baked into the web image; rebuilding with a different
value requires a new image. `AUDIT_CHAIN_KEY_ID` must exist in
`audit-chain-keys.json`.

## 5. Install host timers and directories

Install the backup, maintenance and restore-drill units exactly as described in
[deploy/README.md](../../deploy/README.md#backups-wal-archiving-and-restore-drills):

```sh
sudo install -d -m 0750 /etc/rs-admin
sudo install -m 0640 deploy/systemd/rs-admin-ops.env.example /etc/rs-admin/ops.env
sudo install -m 0644 deploy/systemd/rs-admin-*.service \
  deploy/systemd/rs-admin-*.timer /etc/systemd/system/
sudo install -D -m 0755 deploy/systemd/notify-failure.sh \
  /usr/local/lib/rs-admin/notify-failure.sh
sudo systemctl daemon-reload
sudo systemctl enable --now rs-admin-backup.timer \
  rs-admin-maintenance.timer rs-admin-restore-drill.timer
```

Set the state and metrics directories in `/etc/rs-admin/ops.env` and create
them with the documented owners. `systemctl list-timers 'rs-admin-*'` must show
all three timers.

## 6. First release

The first release cannot be dry-run because there is no previous tag. Run it
through the release workflow, or directly on the host after the production
environment approval:

```sh
./scripts/release.sh --tag <tag> --operator "$USER"
```

The script pulls the immutable images, runs the one-shot migration, starts the
database and long-running services, waits up to 120 seconds for `/health/ready`,
runs the edge smoke checks, and writes a release record under
`$RELEASE_STATE_DIR`. A failure before the stack is healthy exits non-zero; the
record names the failing stage.

If the stack starts but the API is not ready, check:

```sh
docker compose -f deploy/compose.yaml ps
docker compose -f deploy/compose.yaml logs --tail=200 api
docker compose -f deploy/compose.yaml logs --tail=200 migrate
```

## 7. Bootstrap the first super administrator

The database must be migrated and the API image must contain the management
binaries. The command runs as the runtime role, which has the required DML and
audit grants. The password is read twice without echo and shown once.

```sh
cd /opt/rs-admin
docker compose -f deploy/compose.yaml --profile local-db --profile tools \
  run --rm --no-deps -it \
  --entrypoint /usr/local/bin/rs-admin-bootstrap-super-admin api \
  --email ops@example.com --username ops.admin
```

The command refuses to run while an enabled super administrator exists. Record
the user id and the temporary password expiry in the deployment ticket, deliver
the temporary password to the operator over an out-of-band channel, and require
a password change at first login.

For an existing super administrator whose credentials are lost, use
[break-glass.md](./break-glass.md) instead.

## 8. Verification (acceptance)

Run every check and record the output.

```sh
cd /opt/rs-admin

# 1. Only the gateway publishes host ports; all health checks are healthy.
docker compose -f deploy/compose.yaml ps
docker compose -f deploy/compose.yaml --profile local-db --profile tools \
  run --rm --no-deps --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api

# 2. Public edge: SPA 200, unauthenticated API 401 problem+json.
curl --cacert <CA> https://admin.example.com/                       # 200
curl --cacert <CA> https://api.example.com/api/v1/auth/me            # 401
curl --cacert <CA> https://api.example.com/health/ready              # 404 (internal only)

# 3. Internal readiness.
docker compose -f deploy/compose.yaml exec api \
  curl -fsS http://127.0.0.1:8080/health/ready

# 4. The bootstrap event is on the audit chain.
docker compose -f deploy/compose.yaml exec db psql -U rs_admin -d rs_admin -c \
  "SELECT action, actor_label, target_label, occurred_at
     FROM audit_logs
    WHERE action = 'system_bootstrap_super_admin'
    ORDER BY chain_seq DESC LIMIT 1"

# 5. A full backup succeeds and the metric is written.
docker compose -f deploy/compose.yaml --profile local-db --profile tools run --rm backup
grep pgbackrest_backup_last_attempt_success \
  /var/lib/rs-admin/metrics/rs-admin-backup.prom

# 6. Start the observability stack and confirm Prometheus scrapes every target.
docker compose -f deploy/compose.observability.yaml up -d
docker compose -f deploy/compose.observability.yaml exec prometheus \
  wget -qO- 'http://localhost:9090/api/v1/targets?state=active' | grep -o '"health":"up"' | wc -l
```

Then complete the interactive checks:

1. Open `https://admin.example.com/`, sign in with the bootstrap temporary
   password, and change it. The dashboard must load.
2. Open Users, Roles and Audit Logs; the bootstrap event is visible with actor
   `cli`.
3. Sign out and sign in again with the new password.
4. Confirm the external uptime provider reports both probe locations healthy
   and that its notification channel is independent of this host.
5. Send a test alert with Alertmanager and confirm the configured chat channel
   receives it (an empty `alert_webhook_url` intentionally skips this):
   ```sh
   docker compose -f deploy/compose.observability.yaml exec alertmanager \
     amtool --alertmanager.url=http://localhost:9093 alert add \
     alertname=DeploymentTest severity=warning
   ```

A first deployment is accepted only when every command above succeeds and the
release record, bootstrap details and alert test are attached to the
deployment ticket. Open a ticket for any failed check; do not hand the system
to users with an unverified backup or alert path.
