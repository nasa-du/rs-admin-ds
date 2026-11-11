# ADR-0008: 部署、可观测与 CI

- Status: accepted
- Date: 2026-11-07

## Context

「部署、可观测与 CI（含 PG18 升级预留、容量/SLO）」票据需要把此前 ADR 留给部署层的空白一次补齐：实际部署拓扑、域名/证书/网关、Axum 运行时配置、结构化日志与指标、健康检查、CI 流水线、密钥与 TLS、容量/SLO、多实例约束、备份/恢复与 PostgreSQL 18 升级路径。

既定约束：

- ADR-0004 §10 明确把「实际域名/证书/网关、健康检查与 metrics、CI、容量/SLO、审计日志保留与清理」交给本票据；迁移必须作为独立部署步骤、runtime role 无 DDL 权限。
- ADR-0005 §6.3.8 把外部 WORM/日志外发、§7.2 把 HSTS `preload`、§8.1 把 secret manager、§9.8 把应用日志保留期交给本票据。
- ADR-0006 §2 固定前端通过 `VITE_API_BASE_URL` 注入 API 地址；ADR-0007 §8 固定 bun 1.4.2 构建、静态运行时不含 Node/bun。
- 技术栈固定：Rust + Axum + SQLx + PostgreSQL 18；不引入 Redis；多实例是设计前提，但首发规模小；无邮件服务。

本 ADR 只交付规格与决策，不写生产代码。

## Decision

### 1. 部署拓扑与主机基线

1. **单台 Linux 主机 + Docker Compose**，项目名 `rs-admin`。服务划分：
   - `gateway`：nginx，终止 TLS、按 Host 路由、边缘限流与 HSTS；只发布 `80/443`。
   - `web`：多阶段构建的静态前端运行时（`oven/bun:1.4.2` 构建 → `nginx:1.29.8-alpine` 运行），只在 Compose 内网可达。
   - `api`：Rust/Axum 镜像，只在 Compose 内网监听 `8080`（业务）与 `9100`（metrics）。
   - `db`：`postgres:18.6-alpine`，Compose profile `local-db`，只在 Compose 内网可达，不发布主机端口。
   - `maintenance` / `backup`：一次性任务容器，profile `tools`，由主机 systemd timer 触发。
   - `observability`：独立 Compose 文件，包含 Prometheus、Alertmanager、Grafana、Loki、Promtail、postgres_exporter、alert webhook 适配器。
2. **数据库可分离**：默认 `COMPOSE_PROFILES=local-db` 启动本地 `db`；切换到外部 PostgreSQL 时移除该 profile，并在 `.env` 设置 `DATABASE_URL` 或 `DATABASE_URL_FILE`。迁移、健康检查、备份职责随 `DATABASE_URL` 指向切换；外部数据库的备份、小版本补丁与大版本升级由该实例的运维方负责，规格只记录最低保证（PG18、SCRAM、TLS `verify-full`、RPO/RTO）。
3. **主机基线**：4 vCPU / 8 GiB RAM / 100 GiB SSD；Ubuntu 24.04 LTS 或 Debian 12；Docker Engine 27+（Compose v2）；systemd + chrony；时区 UTC。低于 4 GiB RAM 或 2 vCPU 视为容量复查条件。
4. **主机加固清单**：SSH 仅密钥登录并限制来源；防火墙只放行 22（管理员来源）与 80/443；启用自动安全更新；Docker 日志驱动 `json-file` 限制 `max-size=10m`、`max-file=5`；`/etc/rs-admin` 与数据卷权限最小化；容器以非 root 运行，API/网关启用 `no-new-privileges`，API 根文件系统只读（`/tmp` 用 tmpfs）。
5. **网络**：固定 Docker bridge 子网（如 `172.28.0.0/16`），`TRUSTED_PROXY_CIDRS` 覆盖该子网或网关容器地址；`api`/`db`/`web` 不发布主机端口；Prometheus 通过 Compose 内网抓取 `api:9100`。
6. **首发单副本**：API 1 个副本；数据库单实例（本地）或外部单实例。多实例就绪是设计约束，不是首发验收项；扩到 2+ 副本的 checklist 见 §10。

### 2. 域名、网关、TLS 与静态托管

1. **域名占位**：规格继续使用 `admin.example.com`（前端）与 `api.example.com`（API），两者同属同一 registrable domain，保持 ADR-0001 的同站子域假设。上线前必须完成「域名/DNS/证书/网关归属」检查清单：
   - DNS A/AAAA 指向部署主机；`80/443` 对 ACME 与用户可达；
   - 前端与 API 仍同站；若无法同站，按 ADR-0001 回退 BFF，不把 `SameSite=None` 第三方 Cookie 作为生产主方案；
   - 更新 `ADMIN_ORIGIN`、`CORS_ALLOWED_ORIGINS`、`PUBLIC_API_ORIGIN`、CSP `connect-src`、`VITE_API_BASE_URL` 与 HSTS `preload` 决策；
   - 记录证书签发方式、到期时间与续期责任人。
2. **网关选型 nginx**：`limit_req` 原生满足 ADR-0005 §3.5 的 `/auth/*` 每 IP 60 次/分钟粗粒度限流；Caddy 需要第三方插件才能内置限流，v1 不选。
3. **TLS**：TLS 1.2/1.3，现代密码套件；HTTP 一律 301 到 HTTPS；HSTS `max-age=31536000; includeSubDomains`，`preload` 待实际域名确认后追加；证书用 Let's Encrypt + certbot（ACME HTTP-01，webroot 挂载到网关），systemd timer 自动续期，续期后 `nginx -s reload`；证书到期告警见 §6。
4. **路由与代理**：
   - `admin.example.com` → `web:80`；`api.example.com` → `api:8080`。
   - 网关覆盖而非追加 `X-Forwarded-For`（`proxy_set_header X-Forwarded-For $remote_addr;`），设置 `X-Forwarded-Proto`、`Host`；忽略客户端提供的 `X-Real-IP`/`Forwarded`；应用侧只信任 `TRUSTED_PROXY_CIDRS` 内的 peer。
   - 代理超时 30 秒（connect/read/send），与 API 请求超时一致；认证端点 `client_max_body_size 1k`，其他路径 `64k`。
   - `/auth/*` 使用 `limit_req_zone $binary_remote_addr zone=auth:10m rate=60r/m;` + `burst=10 nodelay`，超限返回 429 并带 `Retry-After`；应用层账号/IP/全局限流仍是权威。
5. **静态前端**：
   - 构建阶段 `oven/bun:1.4.2`，`bun install --frozen-lockfile` + `bun run build`；`VITE_API_BASE_URL` 作为 build arg 注入（生产占位 `https://api.example.com/api/v1`）。
   - 运行阶段 `nginx:1.29.8-alpine`，不含 Node/bun；SPA fallback `try_files $uri /index.html`。
   - 缓存：带 hash 的静态资源 `Cache-Control: public, max-age=31536000, immutable`；`index.html` 与错误页 `no-cache`；启用 gzip/brotli。
   - CSP 在 `web` 静态服务配置，按 ADR-0005 §7.3 原文；不得放宽 `script-src` 的内联限制。CSP 由网关或 `web` 统一输出，二选一但只能有一处，避免重复头。
6. **生产不暴露**：`/metrics`、`/health/*`、`/api-docs/*` 与 Swagger UI 不通过公网网关暴露；`/metrics` 仅在 Compose 内网可达，`/api-docs` 生产关闭。

### 3. 配置、密钥与数据库分离

1. **非敏感配置**：`.env`（从 `.env.example` 复制，权限 `0640`），包含端口、域名、连接池、超时、限流阈值、保留期、镜像 tag 等；不得包含密钥。
2. **密钥交付**：主机目录 `/etc/rs-admin/secrets/`（`0400`，root 所有）；Compose 顶层 `secrets:` 从该目录取文件，挂载到容器 `/run/secrets/<name>`。应用优先读取 `*_FILE` 环境变量，环境变量明文作为回退（仅开发/测试）。
3. **必需 secret 文件**：按 ADR-0005 §8.2 的 JWT/refresh 派生/throttle/审计链密钥与数据库连接；新增：
   - `DATABASE_URL_FILE`（外部数据库或本地数据库 DSN，含 `sslmode=verify-full`）；
   - `INTERNAL_CA_CERT_FILE`（PostgreSQL 内部 CA）；
   - `ALERT_WEBHOOK_URL_FILE`（飞书/钉钉/通用 webhook）；
   - `PGBACKREST_REPO_CIPHER_PASS_FILE` 与 S3 兼容存储凭据（仅本地数据库 + backup profile）。
4. **启动校验**：沿用 ADR-0005 §8.3；额外校验 metrics 端口、`TRUSTED_PROXY_CIDRS`、`ALERT_WEBHOOK_URL` 可解析（为空允许但告警），以及当 `COMPOSE_PROFILES` 不含 `local-db` 时 `DATABASE_URL` 必须指向非本机地址。
5. **轮换**：按 ADR-0005 §8.4；轮换后重启 `api`，写 `crypto_key_rotated` 审计。数据库密码轮换需要同时更新 secret 文件与 PG 角色密码，步骤写入部署手册。
6. **数据库分离切换**：改 `.env`（`DATABASE_URL[_FILE]`）+ 移除 `local-db` profile → 重建 `api`/`migrate`/`maintenance`；迁移仍由一次性 `migrate` 容器执行；本地 `db` 卷保留但不再挂载。

### 4. Axum 运行时配置

1. **监听与端口**：业务 `0.0.0.0:8080`；metrics `0.0.0.0:9100`；仅 Compose 内网可达。两个 listener 独立，metrics 端点不经过业务中间件。
2. **连接池**：`DATABASE_MAX_CONNECTIONS=10`/API 副本、`DATABASE_MIN_CONNECTIONS=1`、`acquire_timeout=3s`、`idle_timeout=10m`、`max_lifetime=30m`；PostgreSQL `max_connections=100`。容量公式：`副本数 × 10 + 迁移/维护/备份/监控连接 < 100`。
3. **超时与请求限制**：
   - 请求总超时 30 秒（`tower_http::timeout::TimeoutLayer`）；网关 30 秒；header 读取超时 10 秒；keep-alive 60 秒。
   - 请求体：认证端点 1 KiB，其他 JSON 端点 64 KiB；超限 413 `payload_too_large`。
   - 并发：`tower::limit::ConcurrencyLimitLayer` 保护下游（建议 100 in-flight）；Argon2 另有独立信号量。
4. **Argon2 并发**：`PASSWORD_HASH_CONCURRENCY=max(1, min(4, CPU 核数 / p))`，默认上限 4；等待超过 2 秒返回 503 `service_unavailable` + `Retry-After`；容器内存限制 1 GiB（64 MiB × 4 + 运行时余量），CPU 限制 2。
5. **优雅停机**：监听 SIGTERM/SIGINT；停止接受新连接，等待 in-flight 最多 15 秒，随后 `pool.close().await`；Compose `stop_grace_period: 25s`。网关在滚动/重启时短暂 502 可接受。
6. **启动顺序**：加载并校验配置 → 连接数据库 → 同步权限目录与内置角色（advisory lock）→ 安装 metrics recorder 与 upkeep → 启动 listener。**不在启动时执行迁移**（ADR-0004 §9）。
7. **健康检查**：
   - `GET /health/live`：进程存活即 200，不查数据库。
   - `GET /health/ready`：1–2 秒超时执行 `SELECT 1`，并校验 `_sqlx_migrations` 最新版本不低于内嵌 Migrator 的版本（数据库落后未就绪；数据库领先视为就绪，见 2026-11-10 修订）；失败返回 503 `{"status":"unready"}`，不返回 SQL 错误或连接串。Docker healthcheck 与外部 uptime 使用该端点。
8. **request_id**：服务端生成 UUIDv7，忽略客户端传入；写回 `X-Request-Id`；应用日志与审计共用该值作为 trace_id。
9. **CORS**：`CORS_ALLOWED_ORIGINS=https://admin.example.com`；精确 Origin allowlist，不使用通配符/`null`；credentials 仅限 ADR-0005 §7.4 及其修订的端点。

### 5. PostgreSQL 18、安全与备份

1. **镜像固定**：`postgres:18.6-alpine`（2026-11-07 核验的最新 18.x Alpine tag）；不使用浮动的 `18-alpine`。小版本升级 = 改 tag + 重启 + 验证；大版本升级见 §11。
2. **数据与扩展**：主机卷 `pgdata`；`shared_buffers=1GB`、`effective_cache_size=3GB`、`work_mem=8MB`、`maintenance_work_mem=256MB`、`max_connections=100`、`wal_level=replica`、`archive_mode=on`、`archive_timeout=60s`、`max_wal_senders=10`、`synchronous_commit=on`、`timezone=UTC`、`password_encryption=scram-sha-256`。启用 `pg_trgm`（`q` 子串搜索）与 `pg_stat_statements`；`uuidv7()` 为 PG18 内置。
3. **TLS**：PostgreSQL 启用 `ssl=on`，服务端证书由内部 CA 签发，SAN 覆盖 `db`（本地）或外部数据库主机名；API 使用 `sslmode=verify-full` 并挂载 `INTERNAL_CA_CERT_FILE`；禁止 `sslmode=disable/prefer`。`pg_hba.conf` 使用 `hostssl ... scram-sha-256`，禁止 `md5` 与 PG18 `oauth` HBA（SQLx 不支持，见 `docs/research/axum-stack.md` §10.1）。
4. **数据库角色**：
   - `rs_admin_migrator`：schema owner，拥有 DDL 权限，仅 `migrate` 一次性任务使用；
   - `rs_admin_runtime`：DML 权限，无 DDL；`audit_logs` 仅 `INSERT`/`SELECT`，`audit_chain_head` 有 `SELECT`/`UPDATE`；
   - `rs_admin_cleanup`：维护任务使用，拥有保留期清理所需的 `DELETE`（含 `audit_logs`），不用于应用运行；
   - 备份角色：按 pgBackRest 官方要求配置，限定来源地址与所需权限；若必须 superuser，`pg_hba.conf` 只允许备份容器地址并记录为例外。
5. **备份**：pgBackRest stanza `rs-admin`；每日全量 + 持续 WAL 归档到 S3 兼容对象存储（仓库加密，凭据在 secret 文件）；保留 30 天；备份完成后校验退出码并写 `pgbackrest_backup_last_success_timestamp` 指标。外部数据库不启用本地 pgBackRest，备份责任归外部运维方。
6. **恢复**：每季度从异地备份恢复到同机临时 PostgreSQL 容器/临时目录，验证 `SELECT 1`、迁移版本、关键表行数、审计链 HMAC 校验；记录 RTO 实测。RPO ≤ 15 分钟、RTO ≤ 2 小时（§10）。
7. **pgBackRest 运行方式**：维护/备份镜像基于固定版本 PostgreSQL 镜像并安装固定版本 pgBackRest；`archive_command` 调用 `pgbackrest --stanza=rs-admin archive-push %p`；备份任务与 WAL 归档共享同一仓库配置与 secret。具体镜像版本在实现首日锁定并写入部署手册。

### 6. 可观测性、日志与告警

1. **应用日志**：`tracing` + `tracing-subscriber` JSON 输出到 stdout；`EnvFilter::try_from_default_env()`，非法指令启动失败；字段白名单与脱敏沿用 ADR-0005 §9。Docker `json-file` 仅作本地缓冲（10 MB × 5），Promtail 采集到 Loki，保留 14 天；应用日志保留期由本 ADR 固定为 14 天。
2. **网关日志**：nginx JSON access log 输出到 stdout；只记录 `$uri`（不含 query string）、方法、状态、耗时、上游耗时与 request_id（若存在）；不记录 `Authorization`、`Cookie`、`Set-Cookie`、请求体。
3. **PostgreSQL 日志**：`log_min_duration_statement=500ms`、`log_parameter_max_length=0`、`log_parameter_max_length_on_error=0`，避免参数进入日志；日志经 Docker/Promtail 进 Loki。
4. **指标端点**：API 在 `9100` 暴露 Prometheus 文本格式，仅 Compose 内网可达；Prometheus 每 15 秒抓取，保留 30 天。metric 标签只使用 `MatchedPath` 与有限枚举，不带 username/email/IP/request_id。
5. **指标目录（最小集）**：
   - `http_requests_total{method,route,status}`
   - `http_request_duration_seconds{method,route}`（histogram）
   - `db_pool_connections{state}`（active/idle/max）
   - `auth_login_attempts_total{result,reason}`（`bad_credentials`/`locked`/`rate_limited`/`success`）
   - `auth_login_lockouts_total{scope}`（`account`/`ip`/`global`）
   - `auth_refresh_reuse_detected_total`
   - `auth_password_hash_seconds`（histogram）
   - `auth_global_circuit_open`（0/1 gauge）
   - `audit_write_failures_total`
   - `build_info{version,commit}`
   - PostgreSQL 侧由 `postgres_exporter` 提供连接、复制、`pg_stat_archiver_failed_count` 等；主机侧由 `node_exporter` 提供 CPU/内存/磁盘。
6. **告警目录（Prometheus 规则 → Alertmanager）**：
   - `ApiDown`：`up{job="rs-admin-api"} == 0` 持续 2 分钟（critical）。
   - `Api5xxHigh`：5xx 比例 > 5% 持续 5 分钟（critical）；> 1% 持续 30 分钟（warning）。
   - `ApiLatencyHigh`：p95 > 300 ms 持续 10 分钟（warning）；p95 > 1 s 持续 10 分钟（critical）。
   - `LoginLatencyHigh`：登录 p95 > 1 s 持续 10 分钟（warning）。
   - `RefreshReuseDetected`：`increase(auth_refresh_reuse_detected_total[5m]) > 0`（critical）。
   - `LoginLockoutSpike`：`rate(auth_login_lockouts_total[5m]) > 10/60`（warning）。
   - `GlobalLoginCircuitOpen`：`auth_global_circuit_open == 1`（critical）。
   - `AuditWriteFailure`：`increase(audit_write_failures_total[5m]) > 0`（critical）。
   - `DbPoolSaturated`：active/max > 0.8 持续 5 分钟（warning）。
   - `PgArchiveFailure` / `PgDown`：postgres_exporter 指标（critical）。
   - `BackupStale`：`time() - pgbackrest_backup_last_success_timestamp > 93600`（26 小时，critical）。
   - `DiskSpace`：> 80%（warning）/ > 90%（critical）。
   - `CertificateExpiry`：< 14 天（warning）/ < 7 天（critical）。
   - `ExternalUptime`：外部 uptime 服务对 `https://api.example.com/health/ready` 与 `https://admin.example.com/` 的探测失败。
7. **告警投递**：Alertmanager → 轻量 webhook 适配器 → 飞书自定义机器人 webhook（URL 在 secret 文件；适配器同时支持钉钉/企微/通用 webhook）。critical 立即通知，warning 聚合为每日摘要；通知内容包含环境、告警名、实例、开始时间、runbook 链接，不回传敏感值。
8. **外部存活探测**：使用外部 uptime 服务（Healthchecks.io / UptimeRobot / Better Stack 等）独立于部署主机监控 `ready` 与前端首页；主机整体故障时仍能告警。
9. **Dashboards**：Grafana 提供最小三块：认证（登录结果、锁定、刷新复用）、API（QPS、错误率、延迟、连接池）、PostgreSQL/主机（连接、归档、磁盘、备份年龄）。
10. **外部审计归档/WORM**：v1 不做。审计表按 ADR-0005 的 180/365 天分层保留，应用日志 14 天；HMAC 链与数据库权限继续提供数据库层篡改检测。触发条件（合规要求、审计事故）出现时再评估对象存储 Object Lock/外部 WORM。

### 7. 维护任务与恢复/升级演练

1. **调度**：主机 systemd timer（`Persistent=true`、`RandomizedDelaySec`）每日触发一次性 Compose 任务；不引入常驻调度容器。备份 03:00 UTC，维护清理 04:00 UTC，避免重叠。
2. **维护任务内容**：
   - 删除过期/已撤销且超过 30 天的 `auth_refresh_tokens`；
   - 删除超过 30 天且未锁定的 `auth_login_failures` 与 `auth_login_throttle` 行；
   - 按 ADR-0005 §6.3.6 的分层保留期批量删除 `audit_logs`，同一事务写入 `system_audit_logs_purged` 并更新 `audit_chain_head`；
   - 运行 `pgbackrest check`，导出备份年龄指标；
   - 所有任务先获取固定 key 的 `pg_try_advisory_lock`，未获得则退出 0；防多实例/重复触发并发执行。
3. **恢复演练**：每季度一次，按 §5.6 执行并记录耗时；失败必须开修复票据。
4. **升级演练**：每次 PostgreSQL 大版本升级前，用最近备份在临时实例预演 §11 的 `pg_upgrade` 步骤；演练通过才安排生产维护窗口。

### 8. CI（GitHub Actions）

1. **触发**：`pull_request` 与 `push main`；并发组按分支取消旧运行；生产镜像只在 `main` 与 `v*` tag 上推送。
2. **后端 job**：
   - `cargo fmt --all -- --check`；
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`；
   - `cargo test --workspace --all-targets`，使用 `postgres:18.6-alpine` service container（健康检查 `pg_isready`），`DATABASE_URL` 指向 service，`SQLX_OFFLINE=true` 编译期宏走 `.sqlx`；
   - `cargo sqlx prepare --check --workspace -- --all-targets --all-features`（sqlx-cli 0.9.0，与 crate 版本对齐）；
   - 生成 OpenAPI 并与提交的 `openapi.json` 做 `git diff --exit-code` 防漂移；
   - 构建 API Docker 镜像与数据库/备份镜像（不推送；2026-11-10 修订）。
3. **前端 job**：
   - `oven-sh/setup-bun` 固定 `1.4.2` + 缓存；`bun install --frozen-lockfile`；
   - `bun run lint`、`format:check`、`typecheck`、`i18n:check`、`test`、`build`；
   - `bunx playwright install --with-deps chromium`（browser mode 测试）；
   - orval 生成客户端后 `git diff --exit-code`；orval 8.46 在 Bun 1.4.2 不兼容时按 ADR-0004/0006 回退 `openapi-typescript` + `openapi-fetch`；
   - 构建 `web` Docker 镜像（`VITE_API_BASE_URL` 用占位值）。
4. **Compose 校验**：`docker compose config -q`（含 `local-db`/`tools` profile）验证清单可解析；不启动完整栈。
5. **镜像发布**：`main` 推送 `ghcr.io/nasa-du/rs-admin-ds/api:sha-<short>`、`.../web:sha-<short>` 与 `.../db:sha-<short>`（PostgreSQL 18.6 + pgBackRest，2026-11-10 修订）；`v*` tag 额外推 `vX.Y.Z`；生产不使用 `latest`。
6. **定时安全任务**：每周 `cargo audit`、`bun audit`、Trivy 镜像扫描；PR 不因 advisory 数据库波动阻塞，CRITICAL 结果开 issue 并通知。CI 不接触生产 secret。
7. **必需检查**：上述 job 全部通过才允许合并；`main` 开启分支保护。

### 9. 发布、回滚与迁移执行

1. **镜像与审批**：GitHub Actions 构建并推 GHCR 不可变 tag；部署由 `workflow_dispatch` 手动触发，绑定 `production` environment（required reviewer）。生产按 tag/digest 部署，不追 `latest`。
2. **部署步骤**（SSH 到部署主机）：
   1. `docker login ghcr.io`（只读 token 在主机 secret）；
   2. 记录当前 tag 到部署状态文件；更新 `.env` 的 `API_IMAGE`/`WEB_IMAGE`/`DB_IMAGE`；
   3. `docker compose pull`；
   4. `docker compose run --rm migrate`（使用 `rs_admin_migrator` DSN，等待数据库就绪，应用全部 pending 迁移；非零退出则中止发布）；
   5. `docker compose up -d --remove-orphans`；
   6. 最多 120 秒等待 `/health/ready` 返回 200；失败则切回上一 tag 并 `up -d`，保留新 schema（expand/contract），人工决定是否恢复备份。
3. **迁移策略**：只前进、不自动回滚；迁移文件不可修改（SQLx checksum）；采用 expand/contract——先加列/表/索引，代码双读或默认值兼容，废弃列在后续版本删除；破坏性变更前先做一次 on-demand 备份。任何迁移必须先通过 CI 的空库与上一发布 schema 校验。
4. **停机容忍**：单 API 副本的发布预计数十秒不可用，计入 99.5% 月度预算；v1 不做蓝绿/双实例。若未来要求零停机，先扩到 2 副本并让网关重新解析 DNS（§10），再评估滚动发布。
5. **发布后 smoke**：`/health/ready`、未认证 `GET /api/v1/auth/me` 返回 401 + ProblemDetails + CORS 预检、前端首页 200；不在生产创建测试用户或执行真实登录。
6. **发布记录**：tag、commit、迁移列表、回滚 tag、操作者与时间写入部署日志/issue；保留最近 10 个镜像 tag。

### 10. 容量、SLO 与多实例约束

1. **假设**（超过任一项需重新评估本 ADR）：

   | 维度 | 假设 |
   |---|---|
   | 用户总量 | 10,000 |
   | 并发已认证会话 | 50 |
   | 峰值登录并发 | 20（Argon2 信号量串行/限并发） |
   | 可用性 | 99.5%/月（计划维护 ≤ 4 小时/月，计入预算） |
   | 延迟 | 业务 API p95 ≤ 300 ms；登录 p95 ≤ 1 s；刷新 p95 ≤ 200 ms |
   | RPO / RTO | ≤ 15 分钟 / ≤ 2 小时 |
   | 主机 | 4 vCPU / 8 GiB / 100 GiB SSD |

2. **单机风险**：主机是单点故障；99.5% 依赖主机稳定、外部 uptime 告警、异地备份与恢复演练。主机故障时 RTO ≤ 2 小时是目标而非保证，恢复时间取决于备份下载与恢复速度。
3. **Argon2 内存**：64 MiB × 并发上限 4 = 256 MiB；容器限制 1 GiB；登录峰值 20 并发在信号量队列中排队，超过 2 秒返回 503。禁止提高并发上限而不重新计算内存。
4. **连接池**：单副本 10 连接 + 迁移/维护/备份/监控预留 < `max_connections=100`；扩副本时按公式重算，必要时提升 `max_connections` 并评估 PG 内存。
5. **多实例就绪 checklist**（扩到 2+ API 副本时必须逐项验证）：
   - 无进程内会话/限流/锁定状态；全部共享状态在 PostgreSQL；
   - refresh 轮换用 `SELECT ... FOR UPDATE`；清理/备份任务用 advisory lock 单执行；
   - 网关能重新解析 `api` 的多个容器地址（nginx `resolver 127.0.0.11 valid=10s` + 变量 `proxy_pass`），不使用 sticky session；
   - Prometheus 抓取每个实例，指标聚合不重复计数；
   - 每副本 Argon2 并发 × 内存重新核算；日志中 `request_id` 可跨实例追踪。
6. **扩容触发**：用户 > 10,000、并发会话 > 50、登录 > 5 次/秒、数据库 > 50 GB、p95 持续超目标、主机 CPU > 70% 或内存 > 80% 持续 30 分钟；任一触发即复查主机规格/副本数/数据库规格，必要时迁移到多主机或托管 PG。

### 11. PostgreSQL 18 升级预留

1. **固定镜像与版本节奏**：`postgres:18.6-alpine` 为当前固定 tag；小版本升级在 upstream 发布后 30 天内完成，步骤为改 tag → CI 通过 → 备份 → 重启 → 验证；不使用 `18-alpine` 浮动 tag。升级记录写入部署日志。
2. **迁移可重复**：迁移文件 UTC 时间戳前缀、提交到仓库、不可修改；`.sqlx` 离线元数据提交；CI 在空库与上一发布 schema 上各跑一次；`.gitattributes` 固定 `*.sql text eol=lf`。
3. **允许的 PG18 特性**：`uuidv7()`、`pg_trgm`、`pg_stat_statements`；不得使用未来 PG19-only 语法。schema 不承诺 PG17 回退（ADR-0004 §8/§9）。
4. **大版本升级（18 → 19）手册**：
   1. 预检：确认当前 18.x 为最新小版本；读取 PG19 release notes 与 pgBackRest 兼容矩阵；在临时实例用最近备份演练。
   2. 备份：生产全量备份 + WAL 归档完成并验证；记录当前 `_sqlx_migrations` 版本与扩展列表。
   3. 维护窗口：停止 API 与写入；停止 PG18。
   4. `pg_upgrade`：使用包含 PG18 与 PG19 binaries 的固定维护镜像（镜像 tag 写入 runbook）；先 `pg_upgrade --check`，再执行升级。DB < 50 GB 时默认 `--copy` 以保留旧集群回退能力；磁盘/时间不足时才用 `--link` 并接受只能靠备份回退。
   5. 启动 PG19：运行 `ANALYZE`；验证扩展（`pg_trgm`、`pg_stat_statements`）、迁移表、审计链 HMAC、关键表行数；启动 API 并跑 smoke。
   6. 回退：`--copy` 时切回旧数据目录与 PG18 镜像；`--link` 时从备份恢复。验证通过后保留备份至少 30 天再删除旧集群。
   7. 备选：`pg_dumpall`/`pg_restore` 逻辑迁移（更简单、停机更久）；逻辑复制双写切换不在 v1 推荐范围。
5. **演练**：每次大版本升级前在同机临时实例演练完整流程；每季度恢复演练与升级演练可合并执行。

### 12. 本地开发与 Compose 形态

1. **默认开发**：`docker compose --profile local-db up -d db` 只启动 PostgreSQL 18；API 用 `cargo run`，前端用 `bun dev`；Vite proxy 把 `/api/v1` 转发到 `http://localhost:8080`，使浏览器同源，避免本地 CORS/Cookie 噪音。
2. **全栈联调**：`docker compose --profile local-db --profile full up` 启动 `gateway`/`web`/`api`/`db`，用于发布前验证与 E2E；本地可用自签证书或 HTTP localhost，不得为此降低生产 Cookie 属性。
3. **`__Host-refresh` 本地行为**：Chrome/Firefox 在 `http://localhost` 接受 `Secure` Cookie；若某浏览器拒绝，使用 `mkcert` 生成本地证书并以 `https://localhost:5173` 运行 Vite，**不**移除 `Secure`/`__Host-`。接入说明写入 README。
4. **测试数据库**：`#[sqlx::test]` 使用 Compose `db` 或 CI service；每个测试自动建库；开发 fixture 与生产种子严格分离（ADR-0004 §9）。
5. **密钥与 `.env`**：`.env`、`/etc/rs-admin/secrets`、`*.pem`、`*.key` 均 gitignored；提供 `.env.example` 与密钥生成脚本；开发密钥可丢弃，生产密钥不进仓库。
6. **Docker 构建**：`.dockerignore` 排除 `node_modules`、`dist`、`.git`、`target` 与本地缓存；前端构建阶段 `oven/bun:1.4.2`，运行时静态托管；后端多阶段构建，运行镜像不含 Rust 工具链。

### 13. 对既有 ADR 的落实与边界

1. **落实**：ADR-0004 §10（部署/观测/CI/容量）、ADR-0005 §6.3.8（WORM 评估）、§7.2（HSTS preload）、§8.1（secret manager）、§9.8（日志保留）、ADR-0006 §2（`VITE_API_BASE_URL` 生产占位）与 §9（部署/E2E 验证）、ADR-0007 §8（Docker 构建与静态运行时）。
2. **不改变**：令牌/会话模型、RBAC、密码与限流阈值、API 契约、审计事件目录、前端路由与 i18n；本 ADR 只补部署层。
3. **明确不做**：Kubernetes/Kustomize/Helm、WORM 外部归档、永久 staging、蓝绿/双实例、Redis、邮件告警、外部日志 SaaS、服务网格、GitOps、多区域/多活。
4. **复查条件**：实际域名无法同站 → 按 ADR-0001 回退 BFF；容量/SLO 超限或要求多实例/高可用 → 重新评估主机/K8s/托管 PG；合规要求 WORM → 引入对象存储 Object Lock；PG19 发布 → 执行 §11 演练与升级；主机故障事故 → 评估第二主机或托管 PG。

## Amendments

- 2026-11-11：[PG18 升级演练 + 运维 runbook 全集](https://github.com/nasa-du/rs-admin-ds/issues/52) 落实 §7.4 与 §11.4：
  - 新增升级维护镜像 `deploy/db/upgrade/Dockerfile`：把生产 `DB_IMAGE` 的 PG18 `/usr/local` 树复制到 `/opt/pg18` 作为 `--old-bindir`，目标版本为 `postgres:19rc1-alpine`（PG19 GA 后改成最新 `postgres:19.x-alpine`）。旧树的 `pg_config` 用包装脚本返回 `/opt/pg18` 前缀，避免 `pg_upgrade` 误查新版本的库目录。
  - 新增 `scripts/upgrade-drill.sh`：从最近 pgBackRest 备份恢复到临时 PG18 实例并先跑一次审计链/迁移/关键行验证；确认干净关闭；用目标版本 `initdb` 初始化空集群（沿用源的 encoding/locale、`--username` 与应用超管一致，`pg_upgrade` 本身不运行 `initdb`）；执行 `pg_upgrade --check --copy`；重新初始化并执行真正的 `pg_upgrade --copy`；启动 PG19、运行 `ANALYZE` 与 `pg_upgrade` 生成的 `update_extensions.sql`，再验迁移版本、关键行数、扩展与审计链。默认且只支持 `--copy`，不支持 `--link`；各阶段耗时与两次验证结果写入 `UPGRADE_DRILL_STATE_DIR`。2026-11-11 在开发主机用 30.9 MB、1,334 文件的备份实测：restore 6s、initdb 3s、check 1s、copy 4s、总计 18s；该表只作为方法学基线，生产窗口必须以生产数据重跑。
  - 本修订收紧规格 §11.4 的 `--link` 备选：大版本升级不使用 `--link`，因为删除旧数据目录会失去快速回退路径；磁盘不足时扩容数据卷或改用逻辑迁移（`pg_dumpall`/`pg_restore`），并在维护窗口前把 2 倍数据量作为预检门槛。
  - 维护镜像同时安装 `pgbackrest=2.58.0-r0` 并记录其版本；演练不写生产仓库（`stanza-upgrade`/备份会把临时集群的 system id 写入仓库）。生产切换到 PG19 后必须把 `deploy/db/pgbackrest.conf` 的 `pg1-path` 改为 `/var/lib/postgresql/19/docker`，依次执行 `stanza-upgrade`、`check` 与首次全量备份，再解除发布冻结；旧 PG18 备份保留 30 天作为回退链。
  - runbook 全集覆盖 §10.13 的八项：`first-deploy.md`、`ci-cd.md`、`secret-rotation.md`、`backup-restore.md`、`postgres-upgrade.md`、`alerting.md`、`break-glass.md`、`certificate-renewal.md`，每条包含 Verification 段。备份/归档告警的 `runbook_url` 指向 `backup-restore.md#6-alert-response`，证书告警指向 `certificate-renewal.md#certificateexpiry`，其余指向 `alerting.md` 的同名段。
  - API 镜像增加 `rs-admin-bootstrap-super-admin`、`rs-admin-recover-super-admin` 与 `rs-admin-record-key-rotation`，使首次部署、break-glass 与密钥轮换审计只需一次性容器，不要求主机安装 Rust 工具链。`record-key-rotation` 写 critical 级 `crypto_key_rotated` 事件，details 为 `key_type`、`old_kid`、`new_kid`、`reason`。
- 2026-11-10：[定时运维：备份/WAL/恢复演练 + 维护清理任务](https://github.com/nasa-du/rs-admin-ds/issues/50) 落实 §5.5–§5.7、§7，并取代 #48 修订中「`archive_mode` 暂为 `off`」的过渡状态：
  - `db` 镜像固定为 `postgres:18.6-alpine` + `pgbackrest=2.58.0-r0`。PostgreSQL 启用 `archive_mode=on`、`archive_timeout=60s`，`archive_command` 先 source 读取挂载凭据的 `rs-admin-pgbackrest-env`，再执行 `pgbackrest --stanza=rs-admin archive-push %p`。
  - `db` 额外加入仅用于出站的 `storage` 网络（172.30.0.0/16），以便把 WAL 与备份写入 S3 兼容仓库；它不发布端口，生产 HBA 仍只接受 `data` 子网。`backup` 一次性任务只加入该网络。
  - 每日全量备份在 `backup` 一次性容器中运行，以 postgres 用户共享 `pgdata`（只读）与 `pgsocket` 卷，经本地 Unix socket 以 `rs_admin_backup` 连接 `rs_admin` 数据库；仓库使用 AES-256 加密、时间型 30 天保留。备份任务在备份前执行幂等的 `stanza-create` 与 `pgbackrest check`（包含 WAL 归档校验），因此 `pgbackrest check` 与备份年龄指标属于备份任务而非清理容器。备份角色补授 `pg_create_restore_point(text)`（`pgbackrest check` 需要）。
  - 备份成功后原子写入 `pgbackrest_backup_last_success_timestamp`、`pgbackrest_backup_last_attempt_success`、`pgbackrest_backup_last_attempt_timestamp` 的 Prometheus textfile；失败保留上次成功时间、置 attempt=0 并以非零状态退出。
  - 维护任务由 systemd timer 触发一次性容器，使用 `rs_admin_cleanup` 连接；固定 key 的 `pg_try_advisory_lock` 保证单实例，未获锁以 0 退出。备份任务的并发由 pgBackRest 自身的仓库锁串行化，重复触发会明确失败而不是并行写入仓库。维护任务写 `maintenance_last_success_timestamp`、`maintenance_last_attempt_success`、`maintenance_duration_seconds` 与 `maintenance_rows_deleted{kind}` textfile 指标以监控清理延迟与删除量。清理顺序为 refresh token → session → 登录失败/限流 → 审计；审计只删除「第一行仍在保留期内」之前的连续前缀，在同一事务写入 `system_audit_logs_purged`（含 `from_seq`、`to_seq`、`count`、`last_purged_hash`、`retention_days`）并由写入器更新链头。验证器接受该锚点作为链首。
  - 恢复演练从仓库恢复到一次性容器，校验 `_sqlx_migrations`、关键表行数与审计链，记录 `rto_seconds`；季度 timer 触发，失败通过 systemd `OnFailure` webhook 告警。
- 2026-11-10：[CI 全量、GHCR、发布/回滚与一次性迁移](https://github.com/nasa-du/rs-admin-ds/issues/49) 修订 §4.7 与 §9.2：
  - `/health/ready` 从「迁移版本等于内嵌 Migrator」改为「数据库版本不低于内嵌 Migrator」。发布失败时上一 tag 的代码在新 schema 上继续提供服务；expand/contract 保证新增列/表/索引对旧代码兼容。数据库落后仍返回 503。contract 迁移（删除旧列/表/索引）只能在其旧代码不再作为回滚目标后发布，否则回滚会破坏运行时行为。
  - 发布脚本在显式执行一次性 `migrate` 后，用 `up -d --no-deps` 启动常驻服务（本地数据库先单独启动并等待健康）。这避免 `local-db` profile 在一次 `up` 中再次运行 `migrate`，也避免回滚时旧镜像的内嵌 Migrator 因未知的新迁移而报错。`docker compose --profile local-db up` 的手工验收路径仍保留原行为。
- 2026-11-10：[生产镜像与 Compose 全栈](https://github.com/nasa-du/rs-admin-ds/issues/48) 落实并修订 §1.1、§2.5、§3.2 与 §12.2：
  - `gateway` 与 `web` 使用 `nginxinc/nginx-unprivileged:1.29.8-alpine`（同为 nginx 1.29.8），容器内以 uid 101 监听 8080/8443；Compose 把主机 80/443 映射到这两个非特权端口。这是「容器非 root」与只发布 80/443 的直接结果，但保留了 ADR 固定的 nginx 版本。
  - Compose 的 file-based secret 在非 Swarm 模式下保留宿主文件权限，且 `mode` 选项被忽略。因此 API/migrate 读取的 secret 文件为 `root:10001 0440`，gateway TLS 为 `root:101 0440`，db 入口以 root 复制到 postgres 可读的 `/run/rs-admin-secrets` 后降权；`/etc/rs-admin/secrets` 目录为 `root 0711`。这取代 §3.2 的「0400 root 所有」表述，同时保持「挂载文件优先」和最小权限。
  - 生产栈的常驻服务为 `gateway`/`web`/`api`，`db` 由 `local-db` profile 控制，`migrate` 为 `local-db` 与 `tools` 共用的一次性任务；启用 `local-db` 时 Compose 先跑完 `migrate` 再启动 `api`，外部数据库模式仍显式 `--profile tools run --rm migrate`。因此 `docker compose --profile local-db up` 启动 gateway/web/api/db；§12.2 的 `--profile full` 不再需要。迁移仍不在 API 进程启动时执行。
  - 数据库表级授权在迁移之后由 `migrate` 任务以 `rs_admin_migrator` 执行（`deploy/db/grants.sql`），因为角色与表分别由初始化脚本和迁移创建；未来表通过 `ALTER DEFAULT PRIVILEGES` 继承运行时权限。
  - `archive_mode` 暂为 `off`：在没有 `pgbackrest archive-push` 的情况下启用会无限保留 WAL。备份票据（#50）连同 `archive_command`、pgBackRest 与 `rs_admin_backup` 的函数权限一起启用到 §5.2 的最终配置；本票据已创建备份角色并授予 `pg_backup_start`/`pg_backup_stop`/`pg_switch_wal` 的 EXECUTE。
  - 公网不暴露 `/health/*`（§2.6、本票据验收）与 §6.8 的外部 uptime 探测存在冲突：v1 按 §2.6 收敛，外部 uptime 只探测前端首页，`/health/ready` 由内部监控使用。HSTS `preload` 仍待实际域名确认。
- 2026-11-07：[OpenAPI 3.1 与生成 TS 客户端流水线](https://github.com/nasa-du/rs-admin-ds/issues/25) 修订 §2.5 与 §12.1：
  - `VITE_API_BASE_URL` 为 API 源（不含 `/api/v1`），生产占位 `https://api.example.com`；生成的客户端路径已带完整 `/api/v1` 前缀（ADR-0004 2026-11-07 修订、ADR-0006 同日修订）。
  - 开发 Vite proxy 转发 `/api/v1` 与 `/health` 到 `http://localhost:8080`。

## Consequences

- **正面**：单机 Compose 的运维面最小，同时用固定镜像、一次性迁移、备份/恢复与外部探测守住生产底线；数据库可通过 `.env` 分离，未来迁移到托管 PG 或多主机不需要改应用契约；观测栈覆盖 ADR-0005 要求的登录失败、锁定、刷新复用、审计写入失败与延迟；CI 把格式化、lint、测试、迁移、OpenAPI/orval 漂移与镜像构建收口；PG18 升级有固定 tag、可重复迁移与可演练的 `pg_upgrade` 手册。
- **负面**：单主机是单点故障，99.5% 依赖主机与恢复速度；观测栈与应用同机，主机故障时只能靠外部 uptime 告警；PostgreSQL 自管意味着小版本补丁、备份与升级由本团队负责；单 API 副本发布有数十秒停机；Argon2 64 MiB × 4 抬高内存下限；无 WORM，审计链不能抵御同时掌握应用密钥与数据库的攻击者。
- **风险与缓解**：主机丢失 → 异地备份 + 季度恢复演练；备份凭据泄露 → 仓库加密 + secret 文件权限 + 最小权限；证书过期 → 14/7 天告警；告警通道故障 → 外部 uptime 服务独立告警；数据库连接耗尽 → 池上限与饱和告警；部署失败 → 上一 tag 回滚 + expand/contract 迁移。

## Considered Options

- **Kubernetes（首轮曾选，后改为单机 Compose）**：多副本、滚动发布与生态完整，但对当前规模引入控制面、Ingress、证书、存储与清单运维成本；单机 Compose 足够，未来容量/可用性触发时再评估。
- **托管 PostgreSQL**：备份/HA/补丁由厂商兜底，但 PG18 在目标厂商的可用性未定，且本票据要求固定镜像与 `pg_upgrade` 手册；通过 `DATABASE_URL` 保留迁移路径。
- **Caddy**：自动 TLS 更省事，但内置限流需要 `xcaddy` 第三方插件；nginx 原生 `limit_req` 更符合 ADR-0005。
- **Traefik**：动态配置强，单机场景收益低。
- **Docker Swarm / 多节点**：增加编排复杂度，未达到需要多副本的规模。
- **`pg_dump` 每日备份**：实现简单，但 RPO 24 小时，不满足已接受的 15 分钟。
- **仅本地卷快照**：不满足主机故障与异地恢复。
- **WORM 外部归档进 v1**：需要额外对象存储、密钥与运维，当前规模收益低。
- **永久 staging**：单机资源与维护成本不划算；改用临时恢复/升级演练。
- **CI 自动部署**：缺少人工闸门；改为手动 `workflow_dispatch` + environment 审批。
- **主机上构建镜像**：可复现性与溯源差；改为 Actions 构建推 GHCR、主机只拉取。
- **蓝绿/双实例**：单机与低流量下收益不足；发布停机数十秒计入 SLO。
- **SOPS/age 管理密钥**：增加工具链；主机文件 + Compose secrets 已满足「挂载文件优先」。
- **邮件告警**：项目无邮件服务；用飞书/钉钉/通用 webhook。

## References

- [部署、可观测与 CI（含 PG18 升级预留、容量/SLO）](https://github.com/nasa-du/rs-admin-ds/issues/15)
- [生产可用登录系统实施规格（地图）](https://github.com/nasa-du/rs-admin-ds/issues/1)
- [ADR-0001: 认证令牌与会话模型](./0001-auth-token-session-model.md)
- [ADR-0004: API 契约与数据模型](./0004-api-and-data-contract.md)
- [ADR-0005: 安全基线](./0005-security-baseline.md)
- [ADR-0006: 前端认证集成与路由守卫](./0006-frontend-auth-integration.md)
- [ADR-0007: 前端裁剪、i18n 与 bun](./0007-frontend-pruning-and-i18n.md)
- `docs/research/axum-stack.md`（SQLx 离线/迁移、连接池、健康检查、优雅停机、PG18 兼容性）
- `docs/research/auth-security.md`（同站子域/BFF、日志与审计、密钥轮换）
- PostgreSQL 18 文档：`uuidv7()`、`pg_trgm`、`pg_upgrade`、`ssl`、`archive_command`
- pgBackRest 文档：stanza、WAL 归档、保留策略、`pgbackrest check`
- nginx `limit_req`、certbot ACME HTTP-01、Prometheus/Alertmanager、GitHub Actions 文档
- Docker Hub：`postgres:18.6-alpine`、`nginx:1.29.8-alpine`（2026-11-07 核验）
