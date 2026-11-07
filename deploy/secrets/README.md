# Production secrets

Production key material lives outside the repository at
`/etc/rs-admin/secrets/`, owned by root with mode `0400`. Docker Compose
mounts each file into the container at `/run/secrets/<name>`; the application
reads the matching `*_FILE` environment variable first and only falls back to
a plain environment variable in development and test.

Required files:

| File | Content |
|---|---|
| `database_url` | PostgreSQL DSN with `sslmode=verify-full` |
| `jwt_ed25519_private.pem` | Ed25519 PKCS#8 private key (`JWT_EDDSA_PRIVATE_KEY_FILE`) |
| `refresh_derivation.key` | base64, at least 32 bytes (`REFRESH_DERIVATION_KEY_FILE`) |
| `throttle.key` | base64, at least 32 bytes (`THROTTLE_KEY_FILE`) |
| `audit_chain_keys.json` | JSON object `key_id → base64 key`, including the current `AUDIT_CHAIN_KEY_ID` |

Optional rotation files:

- `jwt_ed25519_previous_public.pem` — verifies tokens signed before a JWT
  rotation (`JWT_EDDSA_PREVIOUS_PUBLIC_KEY_FILE`).
- `refresh_derivation_previous.key` — keeps refresh derivation valid during
  the rotation window (`REFRESH_DERIVATION_KEY_PREVIOUS_FILE`).

Audit chain keys must remain in `audit_chain_keys.json` for at least 365 days
so historical rows can still be verified. See spec §8.7 for the rotation
schedule and the `crypto_key_rotated` audit event.

For local development use `scripts/generate-dev-secrets.sh`, which writes to
`deploy/secrets-dev/` (gitignored).
