# Production secrets

Production key material lives outside the repository at
`/etc/rs-admin/secrets/`. Docker Compose mounts each file into the container at
`/run/secrets/<name>`; the application reads the matching `*_FILE` environment
variable first and only falls back to a plain environment variable in
development and test.

The directory itself should be root-owned with mode `0711` (traversable, not
listable). File modes are driven by which container user must read them:

| File | Reader | Mode / owner |
|---|---|---|
| `database_url` | api | `0440 root:10001` |
| `database_url_migrator` | migrate | `0440 root:10001` |
| `internal_ca.pem` | api, migrate, db | `0440 root:10001` |
| `jwt-ed25519-private.pem` | api | `0440 root:10001` |
| `refresh-derivation.key` | api | `0440 root:10001` |
| `throttle.key` | api | `0440 root:10001` |
| `audit-chain-keys.json` | api | `0440 root:10001` |
| `tls_certificate` | gateway (uid 101) | `0440 root:101` |
| `tls_key` | gateway (uid 101) | `0440 root:101` |
| `postgres_admin_password` | db entrypoint (root) | `0400 root` |
| `postgres_server_cert` | db entrypoint (root) | `0400 root` |
| `postgres_server_key` | db entrypoint (root) | `0400 root` |
| `migrator_password` | db entrypoint (root) | `0400 root` |
| `runtime_password` | db entrypoint (root) | `0400 root` |
| `cleanup_password` | db entrypoint (root) | `0400 root` |
| `backup_password` | db entrypoint (root) | `0400 root` |

The group IDs are the container users: `10001` is `rs-admin` in the API image
and `101` is `nginx` in the gateway image. A `0400 root` file cannot be read by
a non-root container process, which is why the API and gateway files are
group-readable.

File contents:

| File | Content |
|---|---|
| `database_url` | `postgres://rs_admin_runtime:...@db:5432/rs_admin?sslmode=verify-full&sslrootcert=/run/secrets/internal_ca` |
| `database_url_migrator` | the same DSN for `rs_admin_migrator` |
| `internal_ca.pem` | PEM CA that signed the PostgreSQL server certificate |
| `postgres_server_cert` / `postgres_server_key` | PostgreSQL server certificate and key, SAN includes `db` |
| `jwt-ed25519-private.pem` | Ed25519 PKCS#8 private key (`JWT_EDDSA_PRIVATE_KEY_FILE`) |
| `refresh-derivation.key` | base64, at least 32 bytes (`REFRESH_DERIVATION_KEY_FILE`) |
| `throttle.key` | base64, at least 32 bytes (`THROTTLE_KEY_FILE`) |
| `audit-chain-keys.json` | JSON object `key_id → base64 key`, including the current `AUDIT_CHAIN_KEY_ID` |
| `tls_certificate` / `tls_key` | gateway certificate and key for `ADMIN_HOST` and `API_HOST` |
| `postgres_admin_password` | bootstrap superuser password (first initialization only) |
| `migrator_password`, `runtime_password`, `cleanup_password`, `backup_password` | role passwords created on first database initialization |

Optional rotation files (not mounted by the default Compose file):

- `jwt_ed25519_previous_public.pem` — verifies tokens signed before a JWT
  rotation (`JWT_EDDSA_PREVIOUS_PUBLIC_KEY_FILE`).
- `refresh_derivation_previous.key` — keeps refresh derivation valid during
  the rotation window (`REFRESH_DERIVATION_KEY_PREVIOUS_FILE`).

Audit chain keys must remain in `audit-chain-keys.json` for at least 365 days
so historical rows can still be verified. See spec §8.7 for the rotation
schedule and the `crypto_key_rotated` audit event.

## Rotation

- Application keys: replace the file, restart `api`, and write the
  `crypto_key_rotated` audit event. Keep the previous key in the rotation file
  until old tokens/derivations have expired.
- Database roles: `ALTER ROLE ... PASSWORD ...` as the bootstrap superuser,
  then update the matching secret file and restart `api` / `migrate`. The
  first-initialization init script does not re-run, so password rotation is an
  explicit operator step.
- Gateway certificate: replace `tls_certificate`/`tls_key` and reload or
  restart the gateway. Certificate renewal and expiry alerting are covered by
  the deployment runbook.

For local development, `scripts/generate-dev-secrets.sh` writes to
`deploy/secrets-dev/` (gitignored) and sets world-readable modes because all
containers run as non-root with different UIDs.
