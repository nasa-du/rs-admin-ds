# Secret rotation

This runbook covers ADR-0005 §8.4, ADR-0008 §3.5 and AC-OPS-10. Every rotation
has a publish order, an overlap period based on the credential's lifetime, and
a verification step. The audit chain records each application-key rotation as
`crypto_key_rotated` with `key_type`, `old_kid`, `new_kid` and `reason`.

Rotation is a maintenance action. Freeze releases during an overlap window so
an automated restart cannot drop the previous-key override before old tokens or
derivations expire. Never put key material in shell history, a ticket, chat or
the release log; use the secret files and the interactive prompt.

## 0. Common preparation

```sh
cd /opt/rs-admin
SECRETS_DIR=/etc/rs-admin/secrets
export SECRETS_DIR
```

Open a rotation ticket with the key type, planned window, operator and the
second approver for a compromise-driven rotation. Take a backup before changing
any key material:

```sh
docker compose -f deploy/compose.yaml --profile local-db --profile tools run --rm backup
```

Record the current key ids (`JWT_EDDSA_KID`, `AUDIT_CHAIN_KEY_ID`) and the
current file checksums in the ticket.

## 1. JWT signing key

The API signs access tokens with `jwt-ed25519-private.pem`. It verifies tokens
signed by the current key and, during a rotation window, by one predecessor
public key (`JWT_EDDSA_PREVIOUS_PUBLIC_KEY_FILE` with
`JWT_EDDSA_PREVIOUS_KID`). Rotation follows publish-before-use:

1. the new public key is trusted for at least one hour **before** it starts
   signing;
2. after the signing switch, the old public key remains trusted for at least
   the access-token lifetime plus the 60-second clock leeway (one hour).

The single API replica still has two short restarts; schedule a low-traffic
window and freeze releases for the overlap.

1. Generate the new key pair and capture the old public key before replacing
   anything. Keep the old private key in the offline backup:
   ```sh
   install -m 0600 /dev/null /tmp/jwt-private.new
   openssl genpkey -algorithm ED25519 -out /tmp/jwt-private.new
   openssl pkey -in /tmp/jwt-private.new -pubout -out /tmp/jwt-public.new
   openssl pkey -in "$SECRETS_DIR/jwt-ed25519-private.pem" -pubout \
     -out /tmp/jwt-public.old
   ```
2. **Publish phase.** Install the *new* public key as the previous key, set
   `JWT_EDDSA_PREVIOUS_KID=<new-kid>` in `deploy/.env`, keep
   `JWT_EDDSA_KID=<old-kid>` and the old private key, and restart with the JWT
   override:
   ```sh
   cat /tmp/jwt-public.new >"$SECRETS_DIR/jwt-ed25519-previous-public.pem"
   chown root:10001 "$SECRETS_DIR/jwt-ed25519-previous-public.pem"
   chmod 0440 "$SECRETS_DIR/jwt-ed25519-previous-public.pem"

   docker compose -f deploy/compose.yaml -f deploy/compose.rotation-jwt.yaml \
     up -d --no-deps api
   ```
   No token carries the new `kid` yet; the process has simply published the key
   that will verify them. Wait at least one hour and confirm a normal login and
   refresh still work.
3. **Switch phase.** Replace the private key in place, set
   `JWT_EDDSA_KID=<new-kid>`, install the *old* public key as the previous key,
   and set `JWT_EDDSA_PREVIOUS_KID=<old-kid>`:
   ```sh
   cat /tmp/jwt-private.new >"$SECRETS_DIR/jwt-ed25519-private.pem"
   shred -u /tmp/jwt-private.new
   chown root:10001 "$SECRETS_DIR/jwt-ed25519-private.pem"
   chmod 0440 "$SECRETS_DIR/jwt-ed25519-private.pem"

   cat /tmp/jwt-public.old >"$SECRETS_DIR/jwt-ed25519-previous-public.pem"
   chown root:10001 "$SECRETS_DIR/jwt-ed25519-previous-public.pem"
   chmod 0440 "$SECRETS_DIR/jwt-ed25519-previous-public.pem"

   docker compose -f deploy/compose.yaml -f deploy/compose.rotation-jwt.yaml \
     up -d --no-deps api
   ```
   New access tokens now carry the new `kid`; old tokens still verify with the
   previous public key. Wait at least one hour (access TTL + leeway).
4. **Retire phase.** Remove the previous key, unset
   `JWT_EDDSA_PREVIOUS_KID`, and restart the base stack:
   ```sh
   rm -f "$SECRETS_DIR/jwt-ed25519-previous-public.pem"
   docker compose -f deploy/compose.yaml up -d --no-deps api
   ```
5. Record the event:
   ```sh
   docker compose -f deploy/compose.yaml --profile tools run --rm --no-deps \
     --entrypoint /usr/local/bin/rs-admin-record-key-rotation api \
     --key-type jwt --old-kid <old-kid> --new-kid <new-kid> --reason scheduled
   ```
   Use `--reason compromise` when the old key may have leaked; in that case skip
   the overlap, rotate immediately, and revoke all sessions so attacker-held
   refresh tokens cannot mint new access tokens.

**Verification:**

```sh
# Old/new key ids appear in the audit chain.
docker compose exec db psql -U rs_admin -d rs_admin -c \
  "SELECT occurred_at, details FROM audit_logs
    WHERE action = 'crypto_key_rotated'
      AND details->>'key_type' = 'jwt'
    ORDER BY chain_seq DESC LIMIT 1"

# The chain still verifies after the rotation event.
docker compose -f deploy/compose.yaml --profile tools run --rm --no-deps \
  --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api

# A fresh login, a page reload (session bootstrap refresh) and an admin action
# all succeed. A token issued before the switch must still work until it
# expires; do not paste tokens into tickets or chat.
```

Rollback:

- Before the signing switch, remove the previous key and restart the base
  stack; the old key never stopped signing.
- After the signing switch, restore the old private key and `JWT_EDDSA_KID`,
  put the new public key back as the previous key (the publish state), and
  restart the override. Tokens signed with the new key fail; clients refresh.
- For a compromise, do not restore the old key. Rotate immediately and revoke
  all sessions.

## 2. Refresh derivation key

`refresh-derivation.key` derives the next refresh token from the previous raw
token. The previous key must remain available for at least the 30-second grace
window; use five minutes for clock and restart tolerance.

1. Copy the current key to the previous-key file:
   ```sh
   cp "$SECRETS_DIR/refresh-derivation.key" \
     "$SECRETS_DIR/refresh-derivation-previous.key"
   chown root:10001 "$SECRETS_DIR/refresh-derivation-previous.key"
   chmod 0440 "$SECRETS_DIR/refresh-derivation-previous.key"
   ```
2. Generate the new key in place, preserving the inode:
   ```sh
   openssl rand -base64 32 | tee "$SECRETS_DIR/refresh-derivation.key" >/dev/null
   chown root:10001 "$SECRETS_DIR/refresh-derivation.key"
   chmod 0440 "$SECRETS_DIR/refresh-derivation.key"
   ```
3. Restart the API with the refresh rotation override:
   ```sh
   docker compose -f deploy/compose.yaml -f deploy/compose.rotation-refresh.yaml \
     up -d --no-deps api
   ```
4. Wait at least five minutes. Verify a refresh within the grace window still
   succeeds and no unexpected `authn_refresh_reuse_detected` audit event occurs.
5. Remove the previous key and restart with the base Compose file:
   ```sh
   rm -f "$SECRETS_DIR/refresh-derivation-previous.key"
   docker compose -f deploy/compose.yaml up -d --no-deps api
   ```
6. Record `--key-type refresh_derivation` with the old/new key ids.

**Verification:** the audit query in section 1 with
`details->>'key_type' = 'refresh_derivation'`; `rs-admin-verify-audit-chain`;
and a successful login followed by a page reload (session bootstrap refresh).

## 3. Throttle HMAC key

Rotating `throttle.key` resets IP and global counters and the unknown-identifier
account counters. Schedule it off-peak and never during a login attack.

1. Generate the new key in place:
   ```sh
   openssl rand -base64 32 | tee "$SECRETS_DIR/throttle.key" >/dev/null
   chown root:10001 "$SECRETS_DIR/throttle.key"
   chmod 0440 "$SECRETS_DIR/throttle.key"
   ```
2. Restart the API without the override:
   `docker compose -f deploy/compose.yaml up -d --no-deps api`.
3. Record `--key-type throttle`. The old/new ids may be the rotation date and
   the key id, for example `throttle-2026-11`.

**Verification:** the audit query with
`details->>'key_type' = 'throttle'`; a failed-login test still returns the
generic credential error and increments the new counters; no audit or login
errors in the API log.

## 4. Audit chain key

The keyring is a JSON object `key_id -> base64 key`. New rows use
`AUDIT_CHAIN_KEY_ID`; every old key must stay in the keyring for at least 365
days so historical rows remain verifiable.

1. Choose a new key id (for example `audit-2027-01`) and add it without
   removing any existing key:
   ```sh
   umask 077
   new_key="$(openssl rand -base64 32)"
   jq --arg id "<new-key-id>" --arg key "$new_key" \
     '. + {($id): $key}' "$SECRETS_DIR/audit-chain-keys.json" \
     >/tmp/audit-chain-keys.json
   cat /tmp/audit-chain-keys.json >"$SECRETS_DIR/audit-chain-keys.json"
   rm -f /tmp/audit-chain-keys.json
   chown root:10001 "$SECRETS_DIR/audit-chain-keys.json"
   chmod 0440 "$SECRETS_DIR/audit-chain-keys.json"
   ```
2. Set `AUDIT_CHAIN_KEY_ID=<new-key-id>` in `deploy/.env` and restart the API.
3. Record `--key-type audit_chain`. Keep the old key in the keyring.

**Verification:**

```sh
# New rows carry the new key id; old rows still verify.
docker compose exec db psql -U rs_admin -d rs_admin -c \
  "SELECT hmac_key_id, COUNT(*) FROM audit_logs GROUP BY hmac_key_id ORDER BY 1"
docker compose -f deploy/compose.yaml --profile tools run --rm --no-deps \
  --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api
```

A chain verification failure after a keyring change means the old key was
removed or corrupted. Restore the keyring from the offline copy; do not rewrite
or purge rows.

## 5. Database role passwords

The roles are created only on first initialization, so every password rotation
is an explicit operator step. Use the bootstrap superuser over the local socket.

1. Generate the new password and update the role and secret file **in the same
   maintenance window**:
   ```sh
   new_password="$(openssl rand -hex 24)"
   docker compose exec -T db psql -U rs_admin -d rs_admin \
     -c "ALTER ROLE rs_admin_runtime PASSWORD '$new_password'"
   printf '%s\n' "$new_password" >"$SECRETS_DIR/runtime_password"
   printf 'postgres://rs_admin_runtime:%s@db:5432/rs_admin?sslmode=verify-full&sslrootcert=/run/secrets/internal_ca\n' \
     "$new_password" >"$SECRETS_DIR/database_url"
   ```
2. Restart only the consumers of that role:
   - `runtime`: `docker compose -f deploy/compose.yaml up -d --no-deps api`;
   - `migrator`: no running service; the next one-shot `migrate` uses the new
     `database_url_migrator`;
   - `cleanup`: update `cleanup_password` and `database_url_cleanup`; the next
     `rs-admin-maintenance.timer` run uses it;
   - `monitor`: update `monitor_password` and restart `postgres_exporter` in the
     observability stack;
   - `backup`: update `backup_password`; pgBackRest uses the local socket, but
     keep the role and secret consistent for future use.
3. Record the rotation in the operations log (role names only, never
   passwords).

**Verification:** connect with each new DSN from a container that mounts the
secret, for example:

```sh
docker compose --profile tools run --rm --no-deps \
  --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api
docker compose -f deploy/compose.observability.yaml restart postgres_exporter
docker compose -f deploy/compose.observability.yaml exec postgres_exporter \
  wget -qO- http://localhost:9187/metrics | grep '^pg_up 1'
```

The runtime verifier reads the migration table and the audit chain; a wrong
password fails immediately. For `cleanup`, run the maintenance job manually and
confirm it exits 0. For `backup`, run `pgbackrest check`.

## 6. Backup repository credentials and cipher pass

- **S3 access key / secret key**: replace `backup_s3_access_key` and
  `backup_s3_secret_key`, keep the old key valid until the next successful
  backup and a `pgbackrest check`, then deactivate the old key at the provider.
  No container restart is needed; the backup job reads the files on every run.
- **Repository cipher pass**: changing the passphrase in place is not supported.
  Create a new bucket/prefix with the new pass, take a full backup, run a
  restore drill against the new repository, switch `BACKUP_S3_BUCKET` and the
  secret, and keep the old repository read-only until its retention window
  ends. Never delete the old cipher pass before the last backup encrypted with
  it expires.

**Verification:** `pgbackrest --stanza=rs-admin check`, a full backup, and a
restore-drill record with `result: success`.

## 7. Alert webhook and Grafana password

- `alert_webhook_url`: replace the file with the new URL and restart the
  `alert-webhook` service in the observability stack; systemd's
  `notify-failure.sh` reads the file on each failure.
- `grafana_admin_password`: replace the file and restart Grafana, or change the
  password in the Grafana UI and then update the file so a recreated container
  does not drift.

**Verification:** send a test alert (see
[first-deploy.md](./first-deploy.md#8-verification-acceptance)) and open
Grafana with the new password.

## Final verification for any rotation

1. `rs-admin-verify-audit-chain` exits 0.
2. The `crypto_key_rotated` event is present with the right `key_type`,
   `old_kid`, `new_kid` and `reason`.
3. Login, refresh, admin actions and the backup job still work.
4. No secret material appears in logs, tickets or shell history; delete
   temporary files with `shred -u`.
