# ADR-0005: 安全基线

- Status: accepted
- Date: 2026-11-05

## Context

「安全基线：密码、限流、审计事件」票据需要在既有的认证/会话模型（ADR-0001）、用户生命周期（ADR-0003）与 API/数据契约（ADR-0004）之上，确定生产可用的密码哈希、密码策略、登录限流与锁定、可信代理、安全响应头/CORS/CSP、密钥管理、审计事件与防篡改、step-up 重新认证，以及日志脱敏。

约束：不引入 Redis；多实例部署；PostgreSQL 18 保存全部共享状态；无邮件服务；同站子域占位部署（`admin.example.com` + `api.example.com`）；管理后台登录频率低但数据敏感。研究依据见 `docs/research/auth-security.md` 与 `docs/research/axum-stack.md`；本 ADR 中的数值是本系统的决策，不是通用推荐。

## Decision

### 1. 密码哈希

1. **算法与参数**：Argon2id v19，`m = 65536 KiB (64 MiB)`、`t = 3`、`p = 4`、输出 32 字节、盐 16 字节；启用 `argon2` crate 的 `parallel` feature。`p` 是 lane 数，不乘内存；不得用提高 `p` 替代提高 `m`。
2. **基准目标**：单次验证在生产硬件上 100–300 ms，不得超过 1 秒。若超过 300 ms，按顺序回退：`p = 1` → `m = 32 MiB`；任何情况下不得低于 OWASP 最低档 `m = 19 MiB`、`t = 2`、`p = 1`。
3. **存储**：`users.password_hash` 保存完整 PHC 字符串（含算法、版本、参数、盐与哈希）；不增加独立的 salt/params/version 列。
4. **升级策略**：登录验证成功后比较 PHC 参数与当前策略；低于当前策略则用当前策略 rehash 并更新该行。旧哈希在用户下次登录前继续有效；不做强制全员重置。
5. **未知标识**：对不存在的用户执行固定 dummy PHC 哈希验证，参数与当前策略一致，用于时间均衡；dummy 哈希在启动时用当前参数生成。
6. **并发与内存**：`PASSWORD_HASH_CONCURRENCY` 默认 `max(1, min(4, CPU 核数 / p))`；用 `tokio::sync::Semaphore` 限制并发哈希；等待超过 2 秒返回 503 `service_unavailable`（带 `Retry-After`）。内存上限为 `64 MiB × 并发数`。
7. **pepper**：v1 不加服务端 pepper。pepper 会使旧哈希无法验证，轮换需要多版本或强制重置，复杂度收益不匹配。
8. **输入上限**：密码最多 128 个 Unicode 码点 / 512 字节；认证端点请求体上限 1 KiB。登录密码超过上限时仍执行 dummy 验证并返回 401 `invalid_credentials`；创建/改密超过上限返回 422 `password_policy_violation`。

### 2. 密码策略

1. **长度与复杂度**：NFC 规范化后 15–128 个 Unicode 码点；不做大小写/数字/符号组合要求；不做周期性强制改密；v1 不做密码历史。
2. **常见/泄漏密码**：构建时内嵌离线列表前 100,000 条（HIBP Pwned Passwords 频率列表或等价来源，如 SecLists top-100000，需兼容的许可证）；列表版本化、随发布更新、不联网查询。密码经 NFC + 小写后与列表精确匹配；命中返回 422 `password_policy_violation`，文案不透露命中的具体来源。
3. **标识关联**：拒绝与 username、email 或 display_name（NFC + 小写后）完全相同的密码。
4. **规范化**：密码不 trim、不截断；NFC 后原样交给 Argon2；长度按码点计。
5. **修改密码**：自助改密要求当前密码与新密码；新密码不得等于当前密码（422 `password_unchanged`）；成功后按 ADR-0003 保留当前会话、撤销其他会话、递增 `token_version`、重签 access token。
6. **临时密码**：系统生成 24 字符 base64url（约 144-bit）；管理员手输的临时密码同样必须通过上述策略；默认 72 小时过期；`must_change_password = true`；密码值永不写日志或审计。
7. **错误码**：`password_policy_violation`（含稳定的内部 reason：`too_short`/`too_long`/`blocklisted`/`equals_identifier`）、`invalid_current_password`、`password_unchanged`。

### 3. 登录限流与账号锁定

限流状态全部存 PostgreSQL，不使用进程内存作为唯一计数；所有窗口判断使用数据库 `now()`。

1. **账号层**
   - 已知用户：`users.failed_login_count`、`users.locked_until`，新增 `users.last_failed_login_at`。
   - 未知标识：`auth_login_failures.key_hash = HMAC-SHA256(THROTTLE_KEY, 规范化标识)`、`failed_count`、`locked_until`、`updated_at`。
   - 阈值与退避：5 次失败锁 30 秒、10 次锁 5 分钟、15 次锁 1 小时；计数在锁定到期后保留，成功登录清零；连续 24 小时无失败自动衰减为 0；计数 ≥15 时每次新失败继续锁 1 小时。不做永久自动锁定。
   - 锁定时仍执行一次 dummy Argon2 验证，返回 429 `login_locked` + `Retry-After`；锁定期间不增加计数。密码正确但用户停用或临时密码过期返回 403（`account_disabled`/`temporary_password_expired`），不增加失败计数。
   - 管理员解锁（`users.unlock`）清除已知用户的账号层计数、`last_failed_login_at` 与锁定，不动 IP/全局计数；管理员重置凭据同样清除账号层计数、`last_failed_login_at` 与锁定。

2. **IP 层**
   - 表：`auth_login_throttle(scope, key_hash, window_start, failures, locked_until, last_failure_at)`；`scope IN ('ip','global')`；`key_hash = HMAC-SHA256(THROTTLE_KEY, 'ip:' || 规范化 IP)`。
   - IPv4 精确匹配；IPv6 按 /64 聚合；未知与已知标识都计数。
   - 阈值：20 次失败 / 10 分钟固定窗口 → 锁 15 分钟。成功登录不重置 IP 计数（与 ADR-0003 一致）。
   - 锁定时不执行 Argon2，直接 429 `rate_limited` + `Retry-After`；锁定期间不增加计数。

3. **全局层**
   - 同一张 `auth_login_throttle` 表，单行 `scope = 'global'`，`key_hash = HMAC-SHA256(THROTTLE_KEY, 'global')`。
   - 告警阈值：200 次失败 / 5 分钟；熔断阈值：1000 次失败 / 5 分钟 → 全站登录/重新认证 429 `rate_limited` 60 秒。阈值可配置。
   - 熔断只作用于登录与重新认证，不影响 refresh、业务 API 或已登录会话。
   - 锁定时不执行 Argon2；锁定期间不增加计数。触发与解除均告警，避免被单点流量打满。

4. **实现约束**
   - 计数写入使用 `INSERT ... ON CONFLICT DO UPDATE ... RETURNING`；禁止 `SELECT count(*)` 后插入。
   - advisory lock 只包住计数/锁定更新，不包 Argon2 验证（避免串行化登录、放大 DoS）。
   - 数据库不可用时认证 fail-closed（与 ADR-0001 一致）。
   - 清理：`auth_login_throttle` 与 `auth_login_failures` 中超过 30 天且未锁定的行由运维任务批量删除；为 `last_failure_at`/`updated_at` 建索引；监控表大小与清理延迟。
   - 不做 IP + 账号组合键；v1 无 CAPTCHA。

5. **重新认证与刷新**
   - 重新认证的密码失败计入账号层与 IP 层计数，并触发相同的锁定/审计。
   - refresh 端点不做每会话计数；由网关对 `/auth/*` 做每 IP 60 次/分钟的粗粒度限制（部署票据配置），应用层依靠刷新轮换、30 秒 grace 与复用检测。

### 4. 客户端 IP 与可信代理

1. 配置 `TRUSTED_PROXY_CIDRS`（逗号分隔的 CIDR）。为空时忽略 `X-Forwarded-For`，只使用 TCP peer 地址。
2. peer 在可信列表内时，从右向左解析 `X-Forwarded-For`，跳过可信代理，取第一个不可信地址作为客户端 IP；全部可信或格式非法时回退到 peer 并记录告警。
3. 只认 `X-Forwarded-For`；忽略 `X-Real-IP` 与 `Forwarded`。网关必须覆盖而非追加该头。
4. IPv4-mapped IPv6 归一化为 IPv4；IP 限流键使用 IPv4 地址或 IPv6 /64；审计保存规范化后的原始 IP。
5. 限流表只保存 `HMAC-SHA256(THROTTLE_KEY, 规范化键)`，不保存明文 IP；审计表保存原始 IP。
6. 生产环境在网关后运行时 `TRUSTED_PROXY_CIDRS` 不应为空；启动时检查并告警（不直接失败，允许直连部署）。

### 5. Step-up 重新认证

1. **要求最近认证的操作**（`now - auth_sessions.authenticated_at <= 300 秒`）：
   - `POST /users`（一律要求；创建用户会签发临时密码，且可原子携带角色）
   - `PATCH /users/{id}`（仅当请求包含 `username` 或 `email`）
   - `PUT /users/{id}/roles`
   - `POST /users/{id}/reset-password`
   - `PUT /users/{id}/status`
   - `DELETE /users/{id}/sessions`
   - `POST /roles`、`PATCH /roles/{id}`、`PUT /roles/{id}/permissions`、`DELETE /roles/{id}`
2. **不要求**：`POST /users/{id}/unlock`、`POST /auth/logout`、`POST /auth/logout-all`、`PATCH /auth/me` 的 display_name、`GET /audit-logs` 与 `GET /audit-logs/{id}`。
3. **权威来源**：`auth_sessions.authenticated_at` 是判定依据；登录、重新认证、自助改密成功时更新为当前时间；refresh 不重置。`users.token_version` 与 `sid` 校验照常执行。
4. **access token**：新增 `auth_time` claim（Unix 秒），签发时从 `auth_sessions.authenticated_at` 复制；refresh 时复制、不重置。服务端 step-up 校验以数据库列为准，claim 仅供前端预判与展示。
5. **端点**：`POST /auth/reauthenticate`，需要 Bearer，请求 `{password}`，成功 200 `{access_token, token_type: "Bearer", expires_in: 900, auth_time}`。不轮换 refresh token、不建新会话、不改变 `sid`/`ver`/`token_version`；不读取 refresh cookie，因此不要求 CSRF 头；受限会话返回 403 `password_change_required`。
6. **失败与审计**：当前密码错误返回 422 `invalid_current_password`，计入账号/IP 失败层；成功重置账号层失败计数（保留 IP 层计数），与登录成功一致；成功/失败写 `authn_reauth_success`/`authn_reauth_fail`。
7. **拒绝**：最近认证过期返回 403 `reauth_required`（新增错误码），`details.max_age = 300`，写 `authz_step_up_denied`。不使用 401，避免前端把它当作令牌过期而触发刷新循环。
8. **顺序**：先做权限/范围校验，再做 step-up 校验，最后执行变更。
9. **修订**：ADR-0001（claims 与会话列）与 ADR-0004（端点、错误码、OpenAPI、schema）按本节修订。

### 6. 审计事件、字段与防篡改

#### 6.1 事件目录

事件命名沿用 ADR-0003；不采用研究文档中 `authn_token_*`/`session_*` 的重复事件。`authn_password_change` 统一为 `authn_password_changed`；`authn_token_reuse` 统一为 `authn_refresh_reuse_detected`。

| action | severity | 触发 |
|---|---|---|
| `authn_login_success` | info | 登录成功（含受限会话） |
| `authn_login_fail` | warning | 密码错误、未知标识、停用、临时密码过期 |
| `authn_login_lock` | warning | 账号层达到锁定阈值（登录或重新认证） |
| `authn_login_rate_limited` | warning | IP/全局层首次触发锁定 |
| `authn_logout` | info | 当前会话登出 |
| `authn_logout_all` | info | 全部会话登出 |
| `authn_refresh_reuse_detected` | critical | grace 外刷新重放，撤销整族 |
| `authn_session_evicted` | info | 第 6 个会话淘汰最旧会话 |
| `authn_password_changed` | info | 自助改密成功 |
| `authn_password_change_fail` | warning | 自助改密当前密码错误 |
| `authn_reauth_success` | info | step-up 重新认证成功 |
| `authn_reauth_fail` | warning | step-up 重新认证密码错误 |
| `authz_step_up_denied` | warning | 最近认证过期导致敏感操作被拒绝 |
| `authz_denied` | warning | 权限、管理范围、自我操作或内置角色不可改的 403 |
| `user_created` | info | 管理员创建用户 |
| `user_updated` | info | 管理员修改显示名/username/email/phone |
| `user_status_changed` | warning | 启用/停用 |
| `user_password_reset` | critical | 管理员重置凭据（`method=generated|manual`） |
| `user_unlocked` | warning | 管理员解锁 |
| `user_roles_changed` | warning / critical | 角色分配；涉及 `admin`/`super_admin` 时 critical |
| `user_sessions_revoked` | warning | 管理员撤销目标用户全部会话 |
| `role_created` | warning | 创建自定义角色 |
| `role_updated` | warning | 修改角色名称/描述 |
| `role_permissions_changed` | critical | 修改角色权限集 |
| `role_deleted` | warning | 软删除角色 |
| `system_bootstrap_super_admin` | critical | CLI 引导首个超级管理员 |
| `system_recover_super_admin` | critical | CLI break-glass 恢复 |
| `crypto_key_rotated` | critical | JWT/refresh 派生/throttle/审计链密钥轮换 |
| `system_audit_logs_purged` | warning | 按保留期清理审计日志 |
| `audit_log_accessed` | info | 查看审计详情（列表查询不记录） |

#### 6.2 字段

- 顶层字段沿用 ADR-0004：`id`、`occurred_at`、`actor_user_id`、`actor_label`、`action`、`target_type`、`target_id`、`result`、`ip`、`user_agent`、`request_id`、`details`；新增 `severity`。`request_id` 即 trace_id；不新增 `trace_id` 列。
- `result` 取值 `success`、`failure`、`denied`：成功动作为 `success`；认证/操作失败为 `failure`；权限、step-up 或账号状态拒绝为 `denied`。
- `actor_label`：用户动作为事件发生时的 username 快照；CLI 为 `cli`；系统任务为 `system`；未知标识的失败登录为 `anonymous`。
- `details` 按事件定义，至少包含：
  - 登录失败：`reason_code`（`bad_credentials`/`account_disabled`/`temporary_password_expired`）；未知标识附 `identifier_hmac`。
  - 登录成功/重新认证：`auth_method=password`、`session_id`、`must_change_password`（登录）；`auth_time`（重新认证）。
  - 登出：`session_id`；全部登出：`sessions_revoked_count`。
  - 锁定/限流：`lock_seconds`、`failure_count`、`source`（`login`/`reauth`）或 `scope`（`ip`/`global`）、`retry_after_seconds`。
  - 刷新复用：`session_id`、`rotation_counter`、`family_revoked=true`。
  - 会话淘汰：`evicted_session_id`、`new_session_id`。
  - 改密：`sessions_revoked_count`；失败附 `reason_code`。
  - 用户创建：`username`、`email`、`role_ids`、`must_change_password=true`、`temporary_password_expires_at`。
  - 用户更新：`changed_fields`（username/email 记录 old/new，其他字段记录字段名）。
  - 状态变更：`from`、`to`、`sessions_revoked_count`。
  - 凭据重置：`method=generated|manual`、`temporary_password_expires_at`、`sessions_revoked_count`。
  - 解锁：`cleared_failure_count`。
  - 角色分配：`added_role_ids`、`removed_role_ids`、`super_admin_involved`；撤销会话：`sessions_revoked_count`。
  - 角色创建：`role_name`、`permission_keys`；角色更新：`changed_fields`；角色权限变更：`added_permission_keys`、`removed_permission_keys`；角色删除：`role_name`、`permission_keys`。
  - 引导/恢复超级管理员：`target_user_id`、`created`/`recovered`、`sessions_revoked_count`。
  - 密钥轮换：`key_type`（`jwt`/`refresh_derivation`/`throttle`/`audit_chain`）、`old_kid`、`new_kid`、`reason`（`scheduled`/`compromise`）。
  - 审计清理：`from_seq`、`to_seq`、`count`、`last_purged_hash`、`retention_days`。
  - 审计访问：`view=detail`、目标审计日志 id。
  - 授权拒绝：`operation`、`reason_code`；step-up 拒绝附 `max_age_seconds`。
- 密码、密码哈希、临时密码、access/refresh token、Cookie、Authorization 头、密钥永不出现在审计中。
- `ip` 为可信代理解析后的规范化地址；`user_agent` 截断到 512 字符；`details` 不包含请求体或 query string。

#### 6.3 存储与防篡改

1. `audit_logs` 增列：
   - `severity text NOT NULL CHECK (severity IN ('info','warning','error','critical'))`
   - `chain_seq bigint NOT NULL UNIQUE`
   - `prev_hash bytea NOT NULL`
   - `row_hash bytea NOT NULL`
   - `hmac_key_id text NOT NULL`
2. 新增单行链表头：

   ```sql
   CREATE TABLE audit_chain_head (
       id smallint PRIMARY KEY DEFAULT 1 CHECK (id = 1),
       last_seq bigint NOT NULL,
       last_hash bytea NOT NULL,
       updated_at timestamptz NOT NULL
   );
   ```

   迁移时插入 `(1, 0, 32 个零字节, now())`。
3. 写入路径：在业务事务内执行 `SELECT last_seq, last_hash FROM audit_chain_head WHERE id = 1 FOR UPDATE`；应用生成 UUIDv7 `id`，使用数据库 `now()` 作为 `occurred_at`；`chain_seq = last_seq + 1`；`prev_hash = last_hash`；`row_hash = HMAC-SHA256(AUDIT_CHAIN_KEYS[hmac_key_id], prev_hash || 0x00 || JCS(row_without_row_hash))`，其中 `JCS` 为 RFC 8785 规范化 JSON，`row` 包含 `hmac_key_id`；插入审计行并更新链表头。事务提交前不得释放锁。规范化时 UUID、时间、`inet` 用字符串，`prev_hash` 用小写十六进制，`details` 用 JCS，数值按 JCS 规则；验证工具按同一规则重算。
4. 审计写入与业务动作同一事务，fail-closed：审计写失败则业务动作失败（500/503）并告警。登录失败等无业务事务的事件同样在返回前写审计；写失败返回 503。
5. 运行角色对 `audit_logs` 只有 `INSERT`/`SELECT`，无 `UPDATE`/`DELETE`；对 `audit_chain_head` 有 `SELECT`/`UPDATE`。清理任务用独立角色，按保留期批量 `DELETE`，并在同一链路写入 `system_audit_logs_purged`，`details` 记录被删区间与最后一行的 `row_hash` 作为验证锚点。
6. 保留期：`info`/`warning` 180 天；`error`/`critical` 365 天；可配置且不得低于该下限。清理批量执行、限制单次删除行数，并监控延迟。
7. `GET /audit-logs` 增加可选 `severity` 过滤；`AuditLogSummary` 增加 `severity`；增加 `(severity, occurred_at DESC)` 索引。
8. 外部隔离存储/WORM/日志外发不在 v1 范围，由「部署、可观测与 CI」票据评估；v1 的 append-only 权限 + HMAC 链只能检测数据库层篡改，不能抵御同时掌握应用密钥与数据库的攻击者，残余风险写入安全评审。
9. 告警：刷新复用、锁定激增、管理员角色变更、密钥轮换失败、审计写入失败、异常地理位置/设备登录；审计访问本身由 `audit_log_accessed` 记录。

### 7. 安全响应头、CORS 与 CSP

1. **API 响应头**（所有 `/api/v1` 响应）：
   - `Cache-Control: no-store`
   - `X-Content-Type-Options: nosniff`
   - `Referrer-Policy: no-referrer`
   - `X-Frame-Options: DENY`
   - `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'; base-uri 'none'`
   - `Permissions-Policy: camera=(), microphone=(), geolocation=()`
2. **网关/TLS 终止层**：`Strict-Transport-Security: max-age=31536000; includeSubDomains`；`preload` 待实际域名确认；HTTP 自动跳转 HTTPS。
3. **前端静态站 CSP**（由部署票据在静态托管/网关执行）：
   `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' https://api.example.com; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'; upgrade-insecure-requests`
   - Vite 构建关闭 modulepreload polyfill 或改用 hash，确保无内联脚本。
   - `style-src 'unsafe-inline'` 是 Radix/shadcn 内联样式的现实需要；不得放宽 `script-src`。
4. **CORS**：
   - 精确 Origin allowlist，不使用通配符，不允许 `null`。
   - 仅 `/auth/login`、`/auth/refresh`、`/auth/logout` 允许 credentials（`Access-Control-Allow-Credentials: true` 且 `Access-Control-Allow-Origin` 为具体 origin）。
   - `Vary: Origin`；`Access-Control-Allow-Headers: Authorization, Content-Type, X-Requested-With, X-Request-Id`；`Access-Control-Expose-Headers: X-Request-Id, Retry-After, WWW-Authenticate`；`Access-Control-Max-Age: 600`。
   - 4xx/5xx 响应同样带正确的 CORS 头；预检返回 204。
5. **CSRF**：沿用 ADR-0001 §6 与 ADR-0004 §6——精确 Origin + POST + `application/json` + `X-Requested-With`，refresh cookie `SameSite=Strict`；业务 API 只用 Bearer、不读 cookie；v1 不做 double-submit 或同步令牌。同站兄弟子域仍属 same-site，因此 Origin 精确匹配是主要控制，不能只依赖 SameSite。

### 8. 密钥与配置管理

1. **交付方式**：生产优先读取挂载的 secret 文件（`*_FILE`），环境变量作为回退；不引入 Vault 客户端，具体 secret manager 由部署票据决定。
2. **必需配置**：
   - `APP_ENV`、`DATABASE_URL`/`DATABASE_URL_FILE`（`sslmode=verify-full`）
   - `PUBLIC_API_ORIGIN`、`ADMIN_ORIGIN`、`CORS_ALLOWED_ORIGINS`
   - `TRUSTED_PROXY_CIDRS`
   - `JWT_ISSUER`、`JWT_AUDIENCE`、`JWT_EDDSA_KID`
   - `JWT_EDDSA_PRIVATE_KEY_FILE`（Ed25519 PKCS#8 PEM）
   - `JWT_EDDSA_PREVIOUS_PUBLIC_KEY_FILE`（可选，验签旧令牌）
   - `REFRESH_DERIVATION_KEY_FILE`、`REFRESH_DERIVATION_KEY_PREVIOUS_FILE`（可选）
   - `THROTTLE_KEY_FILE`
   - `AUDIT_CHAIN_KEY_ID`、`AUDIT_CHAIN_KEYS_FILE`（JSON：`key_id → base64 key`，含当前与旧钥）
   - `PASSWORD_HASH_CONCURRENCY`
   - `LOGIN_IP_FAILURE_LIMIT=20`、`LOGIN_IP_WINDOW_SECONDS=600`、`LOGIN_IP_LOCK_SECONDS=900`
   - `LOGIN_GLOBAL_ALERT_LIMIT=200`、`LOGIN_GLOBAL_CIRCUIT_LIMIT=1000`、`LOGIN_GLOBAL_WINDOW_SECONDS=300`、`LOGIN_GLOBAL_LOCK_SECONDS=60`
   - `ACCOUNT_FAILURE_DECAY_HOURS=24`
   - `TEMP_PASSWORD_TTL_HOURS=72`
   - `AUDIT_RETENTION_INFO_DAYS=180`、`AUDIT_RETENTION_CRITICAL_DAYS=365`
   - `REAUTH_MAX_AGE_SECONDS=300`
3. **启动校验**：文件存在且可读；PEM 可解析且算法为 Ed25519；`kid`/`iss`/`aud` 非空；所有 HMAC/派生密钥 ≥32 字节；CIDR、Origin 可解析且 Origin 为 `https://` 绝对地址、无通配符/`null`；审计链密钥环包含当前 `key_id`；数据库可连接且使用 `verify-full`。失败即退出，错误信息不包含密钥值。
4. **轮换**：
   - JWT：publish-before-use；先发布新公钥并等待至少 1 小时，再切换签名；旧公钥保留 ≥ access token TTL + 时钟偏差（≥1 小时）。
   - refresh 派生密钥：新钥上线后旧钥保留 ≥5 分钟（覆盖 30 秒 grace）。
   - throttle 密钥：轮换会重置 IP/全局计数与未知标识的账号计数，安排在低峰期。
   - 审计链密钥：新 `key_id` 写入 `hmac_key_id`；旧钥在 `AUDIT_CHAIN_KEYS_FILE` 中保留 ≥365 天以验证历史记录。
   - 每次轮换写 `crypto_key_rotated`。
5. **JWT 校验**：固定 EdDSA、`kid` 从本地可信密钥集选择、`iss`/`aud` 必填、`exp`/`nbf` 启用、时钟偏差 60 秒、显式 `typ=at+jwt`；不信任 token 中的 `jku`/`jwk`/`x5u`。v1 不发布 JWKS。
6. **开发环境**：提供 `.env.example` 占位与密钥生成脚本；`.env` 与真实密钥 gitignored；生产不使用明文 `.env`。

### 9. 日志与脱敏

1. 结构化 JSON 输出到 stdout；`tracing` + `tracing-subscriber`；`EnvFilter::try_from_default_env()`，非法指令启动失败。
2. 应用日志字段白名单：`request_id`、`method`、`matched_path`、`status`、`latency_ms`、`user_id`、`error_code`。不记录 query string、请求/响应体、headers。
3. 永不记录：密码、密码哈希、临时密码、access/refresh token、`Authorization`、`Cookie`、`Set-Cookie`、MFA/重置密钥、审计链密钥、数据库 URL。敏感值用 `secrecy` 等类型包裹，`#[tracing::instrument(skip(...))]`，`TraceLayer` 不启用 `include_headers`。
4. username/email 不进入应用日志（用 `user_id`）；IP/User-Agent 只进入审计。错误日志只记录稳定错误码与内部 reason，不记录 SQL、堆栈或请求体。
5. `request_id` 由服务端生成 UUIDv7，忽略客户端传入；通过 `X-Request-Id` 回传；应用日志与审计共用该值作为 trace_id。
6. metrics 标签使用 axum `MatchedPath` 与有限 reason 枚举（如 `bad_credentials`/`locked`/`rate_limited`），不带 username/email/IP。
7. 生产环境关闭 SQLx 的 INFO 查询日志，禁止记录查询参数。
8. 应用日志保留 7–30 天，由「部署、可观测与 CI」票据配置。前端令牌不得写入 localStorage/console；错误上报需脱敏，由前端票据执行。

### 10. 对既有 ADR 与数据模型的修订

- **ADR-0001**：access token claims 增加 `auth_time`；`auth_sessions` 增加 `authenticated_at`；新增 step-up 要求与 `POST /auth/reauthenticate`。
- **ADR-0004**：
  - `users` 增加 `last_failed_login_at`。
  - `auth_sessions` 增加 `authenticated_at`。
  - `audit_logs` 增加 `severity`、`chain_seq`、`prev_hash`、`row_hash`、`hmac_key_id`；新增 `audit_chain_head`。
  - 新增 `auth_login_throttle(scope IN ('ip','global'), ...)`；`auth_login_failures` 与 `users` 账号计数器保留。
  - `AuditLogSummary` 增加 `severity`；`GET /audit-logs` 增加 `severity` 过滤。
  - 新增端点 `POST /auth/reauthenticate`（operationId `reauthenticate`）与错误码 `reauth_required`。
  - 审计事件目录与 `details` schema 以本 ADR §6 为准。
- **ADR-0003**：账号层阈值不变；IP/全局阈值与收缩机制由本 ADR 补充；step-up 操作清单扩展到 `POST /users` 与 username/email 修改；审计事件按本 ADR §6.1 扩展。
- **CONTEXT.md**：新增重新认证、认证时间、登录限流、失败窗口、常见密码列表、审计级别、审计链、可信代理。

## Consequences

- **正面**：Argon2id 参数高于 OWASP 最低档并用 PHC 参数支持平滑升级；密码策略以长度 + 离线 blocklist 为主，符合 NIST；三层限流全部落 PostgreSQL，适配多实例且不依赖 Redis；step-up 用会话级 `authenticated_at` 作为权威，避免 token 年龄误判；审计 HMAC 链在数据库层提供可验证的防篡改；密钥、响应头、CSP 与日志脱敏均有明确契约。
- **负面**：64 MiB × 并发的内存成本需要容量规划；审计链对写入串行化，极端登录失败量下可能成为瓶颈；HMAC 链无法抵御同时掌握应用密钥与数据库的攻击者；step-up 增加高频管理操作的交互成本；分层保留与清理任务增加运维复杂度；全局熔断可被攻击者用来制造短暂登录拒绝；v1 无 CAPTCHA、无外部 WORM、无每会话 refresh 计数。
- **复查条件**：生产基准超过 300 ms 或内存受限 → 按 §1.2 回退；审计写入延迟或队列积压 → 评估批量/异步锚定或外部存储；管理员反馈 step-up 过频 → 调整窗口或操作清单（需修订 ADR）；需要合规级审计隔离 → 在部署票据引入 WORM/日志外发；blocklist 更新流程需要自动化 → 建立版本化构建步骤；出现第二个验签服务 → 发布 JWKS（ADR-0001 复查条件）。

## Considered Options

- **OWASP 最低档 `19 MiB/t=2/p=1`**：内存友好，但对敏感后台系统偏弱；不选，作为回退下限。
- **Argon2id 加 pepper**：增加数据库泄露时的防御，但轮换需要多版本或强制重置；v1 不选。
- **组合复杂度规则与定期改密**：NIST 与 ASVS 不推荐，收益低、用户负担高；不选。
- **在线 HIBP k-anonymity 查询**：列表更新及时，但引入外部依赖、隐私与可用性问题；不选，改用离线列表。
- **统一 `auth_login_throttle` 存账号/IP/全局**：实现集中，但与 ADR-0004 已接受的 `users`/`auth_login_failures` 冲突、需要重构 `locked_until`/`is_locked`；不选，保留既有表并新增 IP/全局表。
- **只做账号层限流**：无法应对分布式撞库；不选。
- **全局层只告警、不熔断**：不会制造拒绝服务面，但分布式攻击下无最后防线；不选，采用高阈值短时熔断 + 告警。
- **不做 step-up**：依赖 15 分钟 access token，无法抵御被盗会话的高危写操作；不选。
- **查看审计日志也要求 step-up**：更严格，但审计审阅频繁重输密码；v1 不选（研究标为 P1）。
- **仅靠数据库权限防篡改**：无法发现 DBA/数据库层篡改；不选，增加 HMAC 链。
- **外部 WORM 作为 v1 前置**：需要额外基础设施；不在本次范围，留作部署复查。
- **环境变量明文密钥**：简单但易泄露到进程环境/日志；不选，优先挂载文件。
- **严格 CSP 禁止 `style-src 'unsafe-inline'`**：Radix/shadcn 需要内联样式，会破坏 UI；不选，保留 `style-src 'unsafe-inline'` 并严格限制 `script-src`。
- **应用日志记录 username/email/IP**：排障方便但扩大 PII 面；不选，统一走审计。

## References

- [安全基线：密码、限流、审计事件](https://github.com/nasa-du/rs-admin-ds/issues/9)
- [ADR-0001: 认证令牌与会话模型](./0001-auth-token-session-model.md)
- [ADR-0002: RBAC 权限模型与授权规则](./0002-rbac-permission-model.md)
- [ADR-0003: 用户生命周期与凭据管理](./0003-user-lifecycle-and-credential-management.md)
- [ADR-0004: API 契约与数据模型](./0004-api-and-data-contract.md)
- `docs/research/auth-security.md`、`docs/research/axum-stack.md`
- RFC 9106（Argon2）、RFC 8785（JCS）、RFC 6750（Bearer Token）、RFC 7807（Problem Details）、RFC 6265bis（Cookies）
- NIST SP 800-63B-4、OWASP ASVS 5.0、OWASP Password Storage Cheat Sheet
