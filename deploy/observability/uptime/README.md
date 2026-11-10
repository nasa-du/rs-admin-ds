# External uptime

The observability stack runs on the same single host as the application, so it
cannot report a host, network or Docker failure by itself. An external uptime
service (Healthchecks.io, UptimeRobot, Better Stack, ...) is the independent
signal. It must be configured outside this repository and must notify a
channel that does not depend on the deployment host.

## What to probe

| Probe | URL | Frequency | Purpose |
|---|---|---|---|
| Public SPA | `https://admin.example.com/` | 1 minute, at least two regions | Detect host, DNS, TLS or gateway failure. |

`/health/ready` is intentionally **not** reachable from the public gateway
(ADR-0008 §2.6). ADR-0008's 2026-11-10 amendment resolves the conflict with
§6.8: v1 exposes only the SPA externally. Readiness is covered internally by
the `ApiUnready` alert, whose blackbox_exporter HTTP probe reaches
`http://api:8080/health/ready` over the Compose edge network; `ApiDown` covers
the metrics listener. Exposing a public readiness endpoint would require a new
ADR; do not add a gateway exception for an uptime provider.

The `ExternalUptime` alert is therefore provider-owned: no rule on this host
can fire when the host or Prometheus is down. Configure the provider to notify
the same Feishu/DingTalk/WeCom bot (or a generic webhook) used by the
Alertmanager adapter. The alert is listed in
`docs/runbooks/alerting.md#externaluptime`; the alert catalog records the
provider ownership as a comment in `prometheus/rules/rs-admin.yml`.

## Example: Healthchecks.io

1. Create two HTTP checks with the URLs above.
2. Set the period to 1 minute, grace to 2 minutes, and enable at least two
   regions.
3. Add a webhook integration pointing at the same chat bot (or the
   `alert_webhook_url` target) and a second integration to a channel that is
   independent of the Docker host. Include the runbook link
   `https://github.com/nasa-du/rs-admin-ds/blob/main/docs/runbooks/alerting.md#externaluptime`
   in the notification template.
4. Record the check IDs and the responsible operator in the deployment
   runbook.
5. Verify the alert path by pausing the check or blocking the host briefly
   during a maintenance window; confirm a notification arrives.

UptimeRobot and Better Stack follow the same shape: HTTP monitor, one-minute
interval, multi-region, alert after two failures, chat webhook integration.

## Relationship to the internal alerts

| Failure | Signal |
|---|---|
| API process down, gateway still up | `ApiDown` (Prometheus, internal) |
| API live but database/migrations unready | `ApiUnready` (blackbox_exporter, internal) |
| PostgreSQL unreachable | `PgDown` (postgres_exporter, internal) |
| Whole host, DNS or TLS down | `ExternalUptime` (external provider) |
| TLS certificate expiring or probe failing | `CertificateExpiry` (blackbox_exporter, internal) |
