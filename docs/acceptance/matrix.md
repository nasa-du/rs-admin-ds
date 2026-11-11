# Acceptance matrix (spec §14)

Status: **Pass** or **Risk accepted** as defined in [README.md](./README.md).
Evidence paths are relative to the repository root. Test names are exact
function names where cited; whole-file citations cover multi-case suites.
Scripts are runnable acceptance harnesses.

## 14.1 Authentication and sessions

| ID | Criterion | Evidence | Status |
|---|---|---|---|
| AC-AUTH-01 | Username/email + password returns 200, 15-minute token, `UserSelf`, exact `__Host-refresh` cookie, `authn_login_success` | `backend/crates/api/tests/auth_flow.rs::username_login_issues_a_fifteen_minute_token_and_host_only_cookie`; `browser_security.rs::the_refresh_cookie_uses_the_exact_host_cookie_attributes`; `e2e/tests/journeys.spec.ts` journey 1 | Pass |
| AC-AUTH-02 | Wrong password and unknown identifier are indistinguishable, dummy Argon2, `authn_login_fail`, no enumeration | `auth_flow.rs::wrong_password_and_unknown_identifier_are_indistinguishable`; `auth_flow.rs::over_long_passwords_pay_the_dummy_verification_and_are_rejected`; `throttle_flow.rs::unknown_identifiers_lock_at_the_account_layer_too` | Pass |
| AC-AUTH-03 | 5/10/15 failures lock 30 s/5 min/1 h, 429 `login_locked` + `Retry-After`, locked attempts do not count | `backend/crates/security/tests/throttle.rs::account_lock_thresholds_match_the_spec`; `backend/crates/store/tests/auth_throttle.rs`; `throttle_flow.rs::five_failures_lock_the_account_and_locked_attempts_do_not_count`; `scripts/security-verify.sh` live threshold check | Pass |
| AC-AUTH-04 | IP 20/10 min and global 1000/5 min limits; refresh/business unaffected; success keeps the IP counter | `backend/crates/config/tests/config.rs::auth_and_audit_settings_use_spec_defaults`; `throttle_flow.rs::twenty_ip_failures_lock_the_address_and_success_keeps_the_ip_counter`; `throttle_flow.rs::the_global_circuit_blocks_login_and_reauth_but_not_refresh_or_business_api` | Pass |
| AC-AUTH-05 | Disabled user: existing session 403 `account_disabled`, refresh 403, relogin 403, re-enable keeps old sessions revoked | `backend/crates/api/tests/refresh_flow.rs`; `user_lifecycle_flow.rs::set_status_disables_sessions_and_reenables_login`; E2E journey 4 | Pass |
| AC-AUTH-06 | Refresh rotates; 30 s grace returns the same successor; replay outside grace revokes the family, critical audit, 401 | `refresh_flow.rs::a_retry_within_grace_returns_the_same_successor_without_a_rotation`; `refresh_flow.rs::reuse_outside_grace_revokes_the_family_and_writes_a_critical_audit`; `scripts/perf.sh` concurrent-refresh check | Pass |
| AC-AUTH-07 | Idle 30 min / absolute 12 h; sixth login evicts the oldest by `last_seen_at`; activity update at most once/60 s | `refresh_flow.rs::refresh_updates_activity_at_most_once_per_minute`; `backend/crates/api/tests/session_flow.rs` | Pass |
| AC-AUTH-08 | Logout revokes only the current session, 204 idempotent; logout-all revokes all, bumps `token_version`, clears the cookie; audits | `session_flow.rs::logout_all_revokes_every_session_and_bumps_the_token_version`; E2E journey 8 | Pass |
| AC-AUTH-09 | Change password: 422 wrong current/policy; keeps current session, re-signs token, revokes others, bumps `token_version`, rotates refresh; old token dies | `backend/crates/api/tests/change_password_flow.rs`; E2E journey 9 | Pass |
| AC-AUTH-10 | Temporary password returned once, 72 h, `must_change_password`; restricted allowlist; expired login 403 | `auth_flow.rs`; `change_password_flow.rs::a_restricted_session_is_limited_to_the_allowlist`; E2E journeys 2 and 5 | Pass |
| AC-AUTH-11 | Step-up older than 300 s: 403 `reauth_required` with `max_age=300`; reauth re-signs without rotating refresh or creating a session; failures counted | `auth_flow.rs::reauthentication_refreshes_auth_time_without_rotating_the_session`; `roles_flow.rs`; `frontend/src/features/auth/reauth/*.test.ts` | Pass |
| AC-AUTH-12 | Auth order JWT → user status → session → token_version → permission → scope → step-up; DB down 503 fail-closed | `auth_flow.rs::authorization_checks_run_in_the_documented_order`; `browser_security.rs::server_errors_keep_the_cors_headers`; `scripts/fault-injection.sh` live DB-down scenario | Pass |

## 14.2 RBAC and authorization

| ID | Criterion | Evidence | Status |
|---|---|---|---|
| AC-RBAC-01 | Exactly 13 permission keys; fixed built-in role sets; super-admin bypass but auth/session/step-up still apply; `user` is the immovable baseline | `backend/crates/domain/tests/permission_catalog.rs`; `backend/crates/domain/tests/rbac_policies.rs::super_admin_bypasses_checks_and_owns_the_whole_catalog` | Pass |
| AC-RBAC-02 | Effective permissions are the role union; role/permission changes apply on the next request; no `token_version` bump or session revoke | `rbac_policies.rs::effective_permissions_are_the_union_of_all_roles`; `user_lifecycle_flow.rs::role_changes_apply_on_the_next_request_without_invalidating_sessions` | Pass |
| AC-RBAC-03 | Non-super-admin cannot grant beyond own permissions (403 `permission_subset_violation`); super-admin exempt | `roles_flow.rs`; `user_lifecycle_flow.rs` | Pass |
| AC-RBAC-04 | Management scope: filtered list with filtered `total`, out-of-scope 403 `out_of_scope`, self always visible | `backend/crates/api/tests/users_flow.rs`; `rbac_policies.rs` | Pass |
| AC-RBAC-05 | Management actions on self are 403 `self_action_forbidden` (disable/demote/reset/roles/revoke/unlock) | `user_lifecycle_flow.rs`; `users_flow.rs`; `frontend/src/features/users/components/user-detail-actions.test.tsx` | Pass |
| AC-RBAC-06 | Granting/revoking `admin`/`super_admin` is super-admin-only; privileged options disabled in the UI | `user_lifecycle_flow.rs`; `frontend/src/features/users/components/assign-roles-dialog.test.tsx` | Pass |
| AC-RBAC-07 | Last enabled super admin cannot be disabled/demoted (409 `last_super_admin`); concurrent changes cannot reach zero | `backend/crates/domain/tests/lifecycle.rs`; `user_lifecycle_flow.rs` concurrent-demotion test | Pass |
| AC-RBAC-08 | Role assignment replaces the set, keeps baseline `user`, rejects unknown/soft-deleted roles 422 `unknown_role`, audits `user_roles_changed` (critical for privileged roles) | `user_lifecycle_flow.rs::replace_roles_keeps_the_baseline_role_and_audits_the_difference`; `backend/crates/audit/tests/catalog.rs` | Pass |
| AC-RBAC-09 | Custom role CRUD: unique name, empty/invalid permissions 422, built-in 403 `role_immutable`, in-use 409 `role_in_use`, soft delete hides/404 | `roles_flow.rs` | Pass |
| AC-RBAC-10 | `GET /roles` unfiltered with `assignable`/`manageable`; `GET /permissions` full catalog + `grantable`; no `assigned_user_count`/`deletable` | `roles_flow.rs::permission_catalog_requires_any_role_permission_and_computes_grantable`; `openapi_contract.rs` | Pass |
| AC-RBAC-11 | `audit.read` sees all logs without scope filtering; opening a detail writes `audit_log_accessed`; list does not | `backend/crates/api/tests/audit_logs_flow.rs`; `frontend/src/features/audit-logs/index.test.tsx` | Pass |
| AC-RBAC-12 | Authz matrix: 403 `forbidden`, 403 `out_of_scope`, 403 `password_change_required`, 401 unauthenticated, 404 missing, no disguise | `roles_flow.rs`; `users_flow.rs`; `change_password_flow.rs` restricted-session matrix | Pass |

## 14.3 API, data and security

| ID | Criterion | Evidence | Status |
|---|---|---|---|
| AC-API-01 | Every error is `application/problem+json` with the closed field set, matching `X-Request-Id`, 401 `WWW-Authenticate`, 429 `Retry-After`, no SQL/stack | `http_contract.rs`; `scripts/security-verify.sh` live header/body checks | Pass |
| AC-API-02 | Pagination default 20/max 200; `page_size>200` 422 `page_size_too_large`; bad sort 422 `invalid_sort` | `users_flow.rs`; `audit_logs_flow.rs` | Pass |
| AC-API-03 | Unknown write fields 400 `unknown_field`; empty PATCH 422 `empty_update`; formats, 201+Location, 204 | `update_me_flow.rs`; `roles_flow.rs`; `session_flow.rs` | Pass |
| AC-API-04 | CORS exact Origin only, no `*`/`null`; credentials only on cookie endpoints; `Vary: Origin`; errors keep CORS; preflight 204 | `browser_security.rs::preflight_from_an_allowed_origin_describes_the_cors_policy`, `error_responses_keep_the_cors_headers`, `an_untrusted_origin_never_receives_cors_grants`; `scripts/security-verify.sh` live CORS matrix | Pass |
| AC-API-05 | CSRF for login/refresh/logout/logout-all; Bearer endpoints ignore cookies; exact refresh cookie attributes | `browser_security.rs`; `scripts/security-verify.sh` live CSRF/cookie checks | Pass |
| AC-API-06 | API security headers on all responses; strict frontend CSP with no inline script; HSTS and HTTP→HTTPS; ops endpoints not public | `http_contract.rs::security_headers_are_present_on_success_and_error`; `scripts/check-deploy-edge.sh`; `scripts/security-verify.sh` | Pass |
| AC-API-07 | Audit write in the business transaction fail-closed; chain recomputable; runtime role cannot UPDATE/DELETE audit rows; purge writes the event and anchor | `backend/crates/audit/tests/chain.rs::audit_failure_rolls_back_the_business_transaction`; `backend/crates/audit/tests/hash.rs`; `deploy/db/grants.sql`; `scripts/fault-injection.sh` live audit-failure scenario | Pass |
| AC-API-08 | Retention info/warning 180 days, error/critical 365 days; query window default 30/max 90 days; range errors | `backend/crates/config/tests/config.rs`; `audit_logs_flow.rs`; `backend/crates/cli/tests/maintenance.rs` | Pass |
| AC-API-09 | Argon2id PHC with policy parameters, rehash on login, password policy, correct failure codes | `backend/crates/security/tests/password.rs`; `auth_flow.rs`; `change_password_flow.rs`; `argon2-2cpu.md` | Pass |
| AC-API-10 | Startup validation of secret files/PEM/kid/iss/aud/key length/CIDR/Origin/DB TLS exits; errors omit secret values | `backend/crates/config/tests/config.rs`; `backend/crates/api/tests/startup.rs` | Pass |
| AC-API-11 | No password/hash/temp password/token/cookie/Authorization/key in logs, audit or metrics; username/email absent from application logs | `logging.rs::application_logs_do_not_contain_identifiers_or_credentials`; `logging.rs::request_logs_are_structured_and_redacted`; `metrics.rs`; `backend/crates/audit/tests/redact.rs`; `scripts/security-verify.sh` live redaction scan | Pass |
| AC-API-12 | Committed OpenAPI and orval client match generated output; operationIds/schemas/errors/headers complete | `backend/crates/api/tests/openapi_contract.rs`; `.github/workflows/backend.yml`; `.github/workflows/frontend.yml` | Pass |

## 14.4 Frontend

| ID | Criterion | Evidence | Status |
|---|---|---|---|
| AC-FE-01 | Login page only identifier + password; code-mapped errors; 429 countdown; copyable `request_id` | `frontend/src/features/auth/sign-in/components/user-auth-form.test.tsx`; E2E journey 1 | Pass |
| AC-FE-02 | Bootstrap single-flight refresh→me; 401 silent; network/5xx retryable; `account_disabled` clears and notifies | `frontend/src/features/auth/auth-session.test.ts`; `frontend/src/api/mutator.test.ts`; `e2e/tests/fault-injection.spec.ts` | Pass |
| AC-FE-03 | Concurrent 401 triggers one refresh, one retry per request, one logout on failure, no proactive refresh | `frontend/src/api/mutator.test.ts`; `auth-session.test.ts` | Pass |
| AC-FE-04 | Guards: sign-in redirect, forced password change, 403, public error pages, on-site redirect only | `route-guards.test.ts`; `route-permissions.test.ts`; `redirect.test.ts` | Pass |
| AC-FE-05 | Permission-gated actions/menu; unmanageable targets disabled with a tooltip; centralized route permissions; direct access 403 | `can.test.tsx`; `sidebar-data.test.tsx`; `route-permissions.test.ts` | Pass |
| AC-FE-06 | Reauth prompt once, retry once, cancel preserves the form, 5-second race, inline errors | `frontend/src/features/auth/reauth/*.test.ts*` | Pass |
| AC-FE-07 | Users list/filter/sort/page URL sync; all lifecycle actions; no delete/bulk; one-time temporary password not cached; dual-permission role entry; deep links | `frontend/src/features/users/index.test.tsx::renders no delete or bulk-selection affordances`; `reset-password-dialog.test.tsx` cache assertion; `users-table.test.tsx`; E2E journeys 2–5 | Pass |
| AC-FE-08 | Roles page; full tri-state matrix; single save PATCH+PUT; partial failure; built-in read-only; delete-409 link; unsaved guard | `frontend/src/features/roles/role-detail.test.tsx`; `permission-matrix.test.tsx`; `role-delete-dialog.test.tsx`; `unsaved-changes.test.tsx` | Pass |
| AC-FE-09 | Audit list/filter/sort/page URL sync; 30-day window; detail re-GET on every open; structured/raw; masking; unknown fallback; no export/edit | `frontend/src/features/audit-logs/index.test.tsx::offers no export or edit affordances`; `audit-mask.test.ts`; `audit-catalog.test.ts` | Pass |
| AC-FE-10 | Settings: profile `display_name` only; account password/logout-all/language; local appearance/notifications/display | `frontend/src/features/settings/*.test.tsx` | Pass |
| AC-FE-11 | i18n default zh-CN, en-US switch/persist, html lang/title/date/number, key alignment, error/audit fallback, LTR | `frontend/src/i18n/i18n.test.ts`; `locale-provider.test.tsx`; `frontend/scripts/check-i18n.ts`; `format.test.ts` | Pass |
| AC-FE-12 | `bun install --frozen-lockfile`, `bun run check`, `bun run build`; no pnpm; LICENSE/THIRD-PARTY-NOTICES; no generated-client drift | `.github/workflows/frontend.yml`; `frontend/package.json`; `frontend/Dockerfile`; `frontend/LICENSE`; `frontend/THIRD-PARTY-NOTICES.md` | Pass |

## 14.5 Deployment, observability and operations

| ID | Criterion | Evidence | Status |
|---|---|---|---|
| AC-OPS-01 | Compose full stack; ports only where expected; non-root/read-only/no-new-privileges; `/health/ready` passes | `scripts/check-deploy.sh`; E2E stack boot; `scripts/perf.sh` acceptance stack | Pass |
| AC-OPS-02 | TLS/HSTS/certbot renewal, HTTP→HTTPS, CSP/cache headers, SPA fallback, ops endpoints hidden | `scripts/check-deploy-edge.sh`; `deploy/gateway/*`; `docs/runbooks/certificate-renewal.md` | **Risk accepted** for live ACME issuance and the real domain/certificate (provider-hosted); the TLS/HSTS/CSP/SPA configuration checks pass |
| AC-OPS-03 | CI backend/frontend/Compose/image jobs; main branch protection | `.github/workflows/backend.yml`; `.github/workflows/frontend.yml`; `.github/workflows/publish-images.yml`; `docs/runbooks/ci-cd.md` | **Risk accepted** for the GitHub branch-protection settings (configured in GitHub, not in the tree); the CI jobs pass |
| AC-OPS-04 | Manual approval → pull → one-shot migrate → up → 120 s ready; rollback to the previous tag; forward-only migrations | `scripts/release.sh`; `scripts/tests/release.sh`; `.github/workflows/release.yml`; `docs/runbooks/ci-cd.md` | **Risk accepted** for the production environment approval/live deploy (external); the release/rollback dry-run passes |
| AC-OPS-05 | Daily full + WAL archive, RPO ≤15 min, 30-day retention, backup-failure alert, restore drill RTO ≤2 h | `deploy/db/backup.sh`; `deploy/db/pgbackrest.conf`; `scripts/restore-drill.sh`; systemd timers; `alerting.md#backupstale` | **Risk accepted**: the tooling and schedule exist, but this environment has no production S3 repository or production-sized backup. The quarterly drill must record `rto_seconds ≤ 7200`; owner: operations; trigger: first production deployment and every quarter. |
| AC-OPS-06 | Complete metrics catalog; alert rules can fire; external uptime; log redaction; three Grafana dashboards | `scripts/check-observability.sh`; `deploy/observability/*`; `deploy/observability/uptime/README.md`; `scripts/security-verify.sh` redaction scan | **Risk accepted** for the external uptime provider account (provider-owned, documented); the stack checks and redaction pass |
| AC-OPS-07 | Maintenance cleans refresh/session/login/throttle/audit rows; advisory-lock single execution; purge event + anchor | `backend/crates/store/tests/maintenance.rs`; `backend/crates/cli/tests/maintenance.rs`; `docs/runbooks/backup-restore.md` | Pass |
| AC-OPS-08 | Multi-instance checklist satisfied; scaling triggers monitored; capacity/SLO recorded | `docs/adr/0008-deployment-observability-ci.md`; spec §10.10–§10.11; `deploy/observability/prometheus/rules/rs-admin.yml` capacity alerts; `docs/runbooks/alerting.md#capacity-review`; `performance.md` | Pass (checklist, monitors and capacity record). The separate issue-level login SLO miss is recorded in [performance-analysis.md](./performance-analysis.md), not in this AC |
| AC-OPS-09 | PostgreSQL fixed at `18.6-alpine`; minor upgrade flow; 18→19 `pg_upgrade --copy` drill with rollback and extension/audit verification | `deploy/db/Dockerfile`; `docs/runbooks/postgres-upgrade.md`; `scripts/upgrade-drill.sh`; `scripts/tests/upgrade-drill.sh` | Pass |
| AC-OPS-10 | All eight runbooks exist with verification steps | `docs/runbooks/README.md`; the eight runbook files | Pass |

## 14.6 Specification and handover

| ID | Criterion | Evidence | Status |
|---|---|---|---|
| AC-DOC-01 | Spec covers §1–§15; referenced ADRs/research exist; terminology matches CONTEXT | `docs/specs/login-system.md`; `docs/adr/0001`–`0008`; `docs/research/*`; `CONTEXT.md` | Pass |
| AC-DOC-02 | §15 conflict table complete; assumptions complete; no unresolved blanks; conditional plans have triggers/defaults | `docs/specs/login-system.md` §15 | Pass |
| AC-DOC-03 | Permission matrix, error catalog, audit-event catalog, revocation matrix and ACs are directly usable with no duplicates/omissions | spec §6/§7/§8/§14; `permission_catalog.rs`; `backend/crates/audit/tests/catalog.rs`; `openapi_contract.rs` | Pass |
| AC-DOC-04 | Eight ADRs and CONTEXT committed; spec at `docs/specs/login-system.md`; implementation PRs cite AC IDs | `docs/adr/`; `CONTEXT.md`; [`.github/pull_request_template.md`](../../.github/pull_request_template.md); `CONTRIBUTING.md` | Pass |

## Residual risks carried into production

| Risk | Why accepted | Owner / trigger |
|---|---|---|
| Argon2 and API/refresh/pool performance were measured on the development host with the production CPU budget, not on the final production host | The harness is deterministic and those measurements are inside target; final confirmation requires the target hardware | Operations; re-run before first deploy and after any CPU-limit change |
| Login 20-concurrency p95 ≤ 1 s is not reachable on the specified 4-vCPU host with m=64 MiB/t=3/p=4 | Measured p95 is 2249.89–2521.65 ms at the production 2-CPU API limit (0–10 × 503 across runs) and 1392–1423 ms at 4 CPUs; the Argon2 work alone is ~4 CPU-seconds. See [performance-analysis.md](./performance-analysis.md) | Operations + security; first production deploy and any host/CPU-limit change. Mitigation: ≥8 vCPU with a 4–8 CPU API limit, 4 × 2-CPU replicas, or an ADR revision of the login SLO; never lower Argon2 below policy. |
| Restore drill has no production S3 repository here | The development environment has no production backup data; the tooling, schedule and alerting exist | Operations; first deployment and quarterly |
| Branch protection and deployment environment approvals are GitHub-side settings | They are not representable in the repository tree; CI workflows and runbooks define the required checks | Repository owner; verify at first release |
| External uptime provider and live ACME issuance depend on the production domain/DNS | Both are documented provider configurations, outside the isolated environment | Operations; verify on the production host |
| PostgreSQL 19 is not GA at this writing | The drill pins `postgres:19rc1-alpine` by design and documents the GA tag update | Operations; update the tag after PG19 GA |
| 2 CRITICAL image findings: `brotli-libs` (web) and `gosu` Go stdlib (db); the third, `zlib1g`, was verified as a minizip false positive and is suppressed in `.trivyignore` | The web image can be upgraded; `gosu` is entrypoint-only and needs a base-image update or an explicit risk acceptance. The interactive environment could not create the public tracking issue; the scheduled security workflow opens it, and the operator command is recorded | Security owner; before the first production release. Fix/accept on the tracking issue; see [security-findings.md](./security-findings.md) |
