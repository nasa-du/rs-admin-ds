# Break-glass super-administrator recovery

`rs-admin-recover-super-admin` re-issues a temporary password for an **existing**
super administrator when normal credential recovery is impossible (all
super-admin credentials are lost, a temporary password expired, or the last
super-admin is locked out). It is a controlled emergency action, not a
backdoor: the target must already hold the built-in `super_admin` role, the
command requires direct database access and an interactive confirmation, and
every recovery is written to the audit chain inside the same transaction.

Use the normal admin UI for an ordinary password reset or a locked account.
Use [backup-restore.md](./backup-restore.md) first if the database is
unavailable; break-glass needs a running PostgreSQL instance with the current
schema.

## 1. Preconditions

- An incident or change ticket exists and a security owner has approved the
  recovery.
- **Two people are present**: the operator running the command and a second
  person who verifies the target email and reads back the confirmation.
- The operator has SSH access to the deployment host and the
  `/etc/rs-admin/secrets` directory.
- At least one user still holds the built-in `super_admin` role. If no such
  user exists, the database was restored without its role rows; stop and
  escalate to data recovery, do not try to promote an arbitrary account.
- The API image is the current release and contains
  `/usr/local/bin/rs-admin-recover-super-admin`.

## 2. Verify the target before touching credentials

Run a read-only query over the local socket as the bootstrap superuser:

```sh
cd /opt/rs-admin
docker compose exec db psql -U rs_admin -d rs_admin -c \
  "SELECT u.id, u.username, u.email, u.status, u.must_change_password
     FROM users u
     JOIN user_roles ur ON ur.user_id = u.id
     JOIN roles r ON r.id = ur.role_id
    WHERE lower(u.email) = lower('<target-email>')
      AND r.code = 'super_admin'"
```

Exactly one row must be returned. If the user is disabled, recovery enables the
account as part of the reset; record why. If the row is missing, stop.

## 3. Run the recovery

The command reads the new temporary password twice without echo, asks the
operator to type the target email as confirmation, and prints the password
once. It then enables the account, clears the account-level lock, revokes every
session and increments `token_version`.

```sh
docker compose -f deploy/compose.yaml --profile local-db --profile tools \
  run --rm --no-deps -it \
  --entrypoint /usr/local/bin/rs-admin-recover-super-admin api \
  --email <target-email>
```

- The command uses the runtime DSN (`database_url`), which has the needed DML
  and audit grants. Do not use the bootstrap superuser password in the
  container.
- The second person verifies the email typed at the confirmation prompt.
- Capture the user id, temporary-password expiry, `sessions_revoked` and
  `token_version` from the output. Do not capture the password itself; deliver
  it to the target over an out-of-band channel.
- If the command reports `not a super administrator` or `user not found`, the
  target is wrong or the database is not the expected cluster. Stop; do not
  retry with another account until the target is re-verified.

## 4. Verification

1. The recovery audit event exists and is the newest event for the target:
   ```sh
   docker compose exec db psql -U rs_admin -d rs_admin -c \
     "SELECT occurred_at, action, actor_label, target_label, details
        FROM audit_logs
       WHERE action = 'system_recover_super_admin'
       ORDER BY chain_seq DESC LIMIT 1"
   ```
   `target_label` is the target username; `details.recovered` is true. The
   event has critical severity.
2. The audit chain still verifies:
   ```sh
   docker compose -f deploy/compose.yaml --profile tools run --rm --no-deps \
     --entrypoint /usr/local/bin/rs-admin-verify-audit-chain api
   ```
3. The target can sign in with the temporary password, is forced to change it,
   and reaches the dashboard. An old session from before the recovery must
   receive `401` on its next request.
4. No other super administrator was modified; confirm their `token_version` and
   status are unchanged.

## 5. After the recovery

- The target changes the password at first login. Treat the temporary password
  as compromised if it was ever pasted into a ticket or chat.
- If the recovery was caused by a suspected credential compromise, revoke the
  other sessions of the target and review the audit log for the preceding
  authentication events. Do not disable the audit writer or the chain.
- Record the incident: reason, target, second approver, command output,
  audit row and verification result. Keep the record with the incident.
- If the wrong user was recovered, do not try to undo the operation by editing
  the database. Disable the unintended account through the admin UI, recover
  the correct one, and record both actions.

## Failure modes

| Symptom | Meaning | Action |
|---|---|---|
| `the database is unavailable or its migration history cannot be read` | database down or schema behind | Follow [backup-restore.md](./backup-restore.md) or run the one-shot migration; do not bypass |
| `user not found` | email mismatch | Re-run the read-only target query; stop |
| `not a super administrator` | target lacks the built-in role | Escalate; do not grant the role directly in SQL outside the audited admin flow |
| command exits before the prompt | missing secret/keyring or wrong image | Check `docker compose config`, the mounted secrets and the API image tag |
| audit chain verification fails afterwards | data integrity incident | Stop writes, preserve the cluster and keyring, escalate |
