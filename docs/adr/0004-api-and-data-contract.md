# ADR-0004: API 契约与数据模型

- Status: accepted
- Date: 2026-11-05

## Context

「API 与数据契约」票据需要固定前端生成客户端所依赖的 REST 契约、错误格式、分页与表示约定、OpenAPI/TS 客户端流水线，以及 users、roles、permissions、role_permissions、user_roles、auth_sessions、auth_refresh_tokens、auth_login_failures、audit_logs 的表结构与迁移/种子策略。ADR-0001 已固定令牌与会话模型，ADR-0002 已固定 RBAC，ADR-0003 已固定用户生命周期；本 ADR 在其上补齐 API 表面与数据契约。系统使用 Rust Axum 0.8.9、SQLx 0.9.0、PostgreSQL 18、utoipa 6 与 orval 8（见 `docs/research/axum-stack.md`）。

## Decision

### 1. API 表面与版本

- 前缀 `/api/v1`；资源使用扁平、复数、kebab-case 路径：`/auth/*`、`/users`、`/roles`、`/permissions`、`/audit-logs`。
- 不设 `/admin` 段；管理能力由权限决定，不由 URL 分类。路径参数名统一 `{id}`；不允许尾斜杠（直接 404）。
- JSON 字段一律 snake_case；多词路径 kebab-case。v1 内只做加法变更，破坏性变更开 `/api/v2`。
- `/health/*`、`/metrics` 与 OpenAPI 文档端点不属于 `/api/v1` 业务契约，由「部署、可观测与 CI」票据定义。

### 2. 错误契约

- 错误响应统一 `Content-Type: application/problem+json`，遵循 RFC 7807：

```json
{
  "type": "urn:rs-admin:problem:validation_failed",
  "code": "validation_failed",
  "title": "Validation failed",
  "status": 422,
  "detail": "One or more fields are invalid.",
  "instance": "/api/v1/users",
  "request_id": "018f...",
  "errors": [
    { "pointer": "/email", "code": "invalid_format", "params": {} }
  ]
}
```

- `type` 为 `urn:rs-admin:problem:<code>`；`code` 是稳定的扁平 snake_case 标识；`title`/`detail` 用英文兜底；服务端不按 `Accept-Language` 变文案，前端按 `code` 映射 zh-CN/en-US。`instance` 为请求路径；`request_id` 与响应头 `X-Request-Id` 一致。
- 字段校验用 422 `validation_failed` + `errors[]`（JSON Pointer，支持嵌套，`params` 为可选占位参数）；JSON 解析失败 400 `invalid_json`；未知请求体字段 400 `unknown_field`。
- 状态码语义：400 语法/未知字段；401 未认证；403 已认证但禁止（权限、范围、自我操作、受限会话、停用、临时密码过期、内置角色不可改）；404 资源不存在；405 方法不允许；409 状态冲突；413 请求体过大；415 媒体类型不支持；422 语义/字段校验；429 锁定或限流；500 内部错误；503 服务不可用。
- 所有 401 带 `WWW-Authenticate: Bearer realm="rs-admin"`；429 带 `Retry-After`；绝不回传内部错误、SQL 或堆栈。
- 错误码目录：`invalid_json`、`unknown_field`、`unsupported_media_type`、`method_not_allowed`、`payload_too_large`、`validation_failed`、`invalid_sort`、`page_size_too_large`、`empty_update`、`invalid_time_range`、`time_range_too_large`、`invalid_credentials`、`access_token_invalid`、`session_invalid`、`refresh_token_invalid`、`refresh_reuse_detected`、`account_disabled`、`temporary_password_expired`、`login_locked`、`rate_limited`、`password_change_required`、`csrf_failed`、`forbidden`、`out_of_scope`、`self_action_forbidden`、`role_immutable`、`permission_subset_violation`、`not_found`、`identifier_taken`、`role_name_taken`、`role_in_use`、`last_super_admin`、`invalid_current_password`、`password_policy_violation`、`password_unchanged`、`unknown_role`、`empty_permissions`、`invalid_permission`、`internal_error`、`service_unavailable`。
- 错误码同步进入 OpenAPI 与前端共享 TS 联合类型；客户端必须容忍未来新增码。

### 3. 表示、分页与筛选

- ID 为小写带连字符 UUID 字符串；时间为 RFC3339 UTC（`...Z`，毫秒精度），数据库为 `timestamptz`；枚举为 snake_case 字符串。
- 响应始终包含字段，缺省用 `null`；请求中省略表示「不修改」，显式 `null` 清空可空字段；写请求拒绝未知字段，未知查询参数忽略。
- 用户更新用 `PATCH /users/{id}` 局部更新；GET 安全；创建返回 201 + `Location`；删除/登出/撤销返回 204。v1 不引入 `Idempotency-Key`，唯一约束防重复。
- 列表统一 offset 分页：`page`（1 起，默认 1）、`page_size`（默认 20，上限 200），响应 `{items, total, page, page_size}`；超过上限返回 422 `page_size_too_large`，不静默截断。后续可按端点差异化上限，但不得超过 200。
- 筛选用每端点显式、有类型的 query 参数，不引入通用 filter DSL；排序用 `sort=field` / `sort=-field`，每端点白名单。

### 4. OpenAPI 与 TypeScript 客户端

- 后端用 utoipa 6 + utoipa-axum 0.3 生成 OpenAPI 3.1；`servers: [{url: "/api/v1"}]`；`/api-docs/openapi.json` 仅内网/开发可达，Swagger UI 仅开发/内网启用，生产关闭。
- tags：`Auth`、`Users`、`Roles`、`Permissions`、`Audit Logs`。operationId 固定为：`login`、`refreshToken`、`logout`、`logoutAll`、`getMe`、`updateMe`、`changePassword`、`listUsers`、`getUser`、`createUser`、`updateUser`、`setUserStatus`、`resetUserPassword`、`unlockUser`、`setUserRoles`、`revokeUserSessions`、`listRoles`、`getRole`、`createRole`、`updateRole`、`setRolePermissions`、`deleteRole`、`listPermissions`、`listAuditLogs`、`getAuditLog`。
- security schemes：`bearerAuth`（HTTP Bearer JWT）与 `refreshCookie`（apiKey cookie `__Host-refresh`，仅文档表达，浏览器自动发送）；`X-Requested-With` 作为 cookie 认证端点的必填 header 参数。
- 统一错误 schema `ProblemDetails`；所有响应文档化 `X-Request-Id`，201 文档化 `Location`，429 文档化 `Retry-After`。
- `openapi.json` 作为构建产物由后端 CI 生成并提交；前端用 orval 8（`client: 'react-query'`、`httpClient: 'axios'`、自定义 mutator 注入 Bearer 并做单飞 401 刷新）生成并提交客户端；CI 用 `git diff --exit-code` 防漂移。orval 8.46 要求 Node >=22.18，需在实际 bun 版本验证；不兼容时退到 `openapi-typescript` + `openapi-fetch`。

### 5. 共享 schema

- `UserSummary`：`id`、`username`、`email`、`display_name`、`phone`（nullable）、`status`、`roles: RoleRef[]`、`must_change_password`、`locked_until`（nullable）、`last_login_at`（nullable）、`created_at`、`updated_at`。
- `UserDetail`：`UserSummary` + `temporary_password_expires_at`（nullable）；不暴露 `failed_login_count`。
- `UserSelf`：`id`、`username`、`email`、`display_name`、`phone`、`status`、`roles`、`permissions: string[]`、`is_super_admin`、`must_change_password`、`temporary_password_expires_at`。
- `RoleRef`：`id`、`name`、`is_builtin`。`RoleSummary`/`RoleDetail`：`id`、`name`、`description`（nullable）、`is_builtin`、`permission_keys: string[]`、`assignable`、`manageable`、`created_at`、`updated_at`。
- `PermissionGroup`：`key`、`display_name_zh`、`display_name_en`、`permissions: PermissionItem[]`；`PermissionItem`：`key`、`display_name_zh`、`display_name_en`、`grantable`。
- `AuditLogSummary`：`id`、`occurred_at`、`actor: {id, label}`、`action`、`target: {type, id}`、`result`、`ip`、`request_id`；`AuditLogDetail` 加 `user_agent`、`details`（开放 object）。
- 列表 envelope 固定 `{items, total, page, page_size}`；单资源不包 envelope。
- 用户校验：username 3–32、`^[a-z0-9][a-z0-9._-]*$`、保留 `admin`/`root`/`system` 等少量名称；email trim + Unicode NFC + 小写、≤254、格式校验；display_name 1–64 Unicode、缺省 username、不唯一；phone ≤32、trim、不唯一、不参与登录；密码 15–128、NFC、blocklist、不截断，具体策略由「安全基线：密码、限流、审计事件」细化。
- 角色校验：自定义角色 name 1–64、活跃角色中不区分大小写唯一、description ≤500、`permission_keys` 至少一项且来自目录；内置 `user`/`admin`/`super_admin` 名称保留。

### 6. 认证端点契约

| 端点 | 鉴权 | 请求 | 成功 | 主要错误 |
|---|---|---|---|---|
| `POST /auth/login` | 无 | `{identifier, password}` | 200 `{access_token, token_type:"Bearer", expires_in:900, user: UserSelf}` + Set-Cookie refresh | 401 `invalid_credentials`；403 `account_disabled`/`temporary_password_expired`；429 `login_locked`/`rate_limited` |
| `POST /auth/refresh` | refresh cookie | `{}` | 200 `{access_token, token_type, expires_in}` + 轮换 Set-Cookie | 401 `refresh_token_invalid`/`refresh_reuse_detected`/`session_invalid` |
| `POST /auth/logout` | refresh cookie | `{}` | 204 + 清 Cookie（幂等） | 403 `csrf_failed` |
| `POST /auth/logout-all` | Bearer | `{}` | 204；撤销全部会话 + 递增 `token_version` + 清 Cookie | 401 `access_token_invalid`；403 `password_change_required` |
| `GET /auth/me` | Bearer | — | 200 `UserSelf` | 401 `access_token_invalid` |
| `PATCH /auth/me` | Bearer | `{display_name}` | 200 `UserSelf` | 422 `validation_failed`/`empty_update` |
| `POST /auth/change-password` | Bearer | `{current_password, new_password}` | 200 `{access_token, token_type, expires_in}` + 轮换 refresh Cookie | 422 `invalid_current_password`/`password_policy_violation`/`password_unchanged` |

- login/refresh/logout/logout-all 要求精确 Origin、`X-Requested-With: XMLHttpRequest`、`Content-Type: application/json`；失败 403 `csrf_failed`。业务 Bearer 端点不读 cookie。
- refresh Cookie：`__Host-refresh; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=43200`，不设 `Domain`；登录设置，刷新轮换，登出/全部登出清除。
- login 返回完整 `UserSelf`（含有效权限），refresh 只返回令牌；`GET /auth/me` 返回裸 `UserSelf`，是用户与权限的权威来源。
- 受限会话（`must_change_password=true`）只允许 `GET /auth/me`、`POST /auth/refresh`、`POST /auth/change-password`、`POST /auth/logout`；其他端点（含 `PATCH /auth/me` 与 `POST /auth/logout-all`）返回 403 `password_change_required`。
- 自助改密：校验当前密码；清除 `must_change_password` 与临时密码过期时间；撤销其他全部会话、递增 `token_version`；保留当前会话并重签 access token，同时轮换当前会话的 refresh token（同族）。管理员重置凭据、停用用户、全部登出仍撤销全部会话并递增 `token_version`。
- refresh grace：30 秒内旧刷新令牌幂等重试，返回当前有效后继令牌；grace 外重放撤销整族。实现采用派生式刷新令牌（见 §8），客户端必须单飞刷新。
- 管理员重置密码：`POST /users/{id}/reset-password`，见 §7；重置结果只在响应中返回一次，绝不写日志/审计。

### 7. Users / Roles / Permissions / Audit 端点

**Users**（全部要求 Bearer；管理动作对自己 403 `self_action_forbidden`，对范围外目标 403 `out_of_scope`）：

| 端点 | 权限 | 请求要点 | 成功 |
|---|---|---|---|
| `GET /users` | `users.read` | `q`、`status`、`role_id`、`is_locked`、`must_change_password`、`page`、`page_size≤200`、`sort` | 200 列表 `UserSummary` |
| `GET /users/{id}` | `users.read` | — | 200 `UserDetail` |
| `POST /users` | `users.create`（带 `role_ids` 时还需 `users.assign_role`） | `{username, email, display_name?, phone?, password?, role_ids?}` | 201 + Location `{user: UserDetail, temporary_password, temporary_password_expires_at}` |
| `PATCH /users/{id}` | `users.update` | `{display_name?, username?, email?, phone?}` | 200 `UserDetail` |
| `PUT /users/{id}/status` | `users.set_status` | `{status: "active"\|"disabled"}` | 200 `UserDetail`；最后一位超管 409 `last_super_admin` |
| `POST /users/{id}/reset-password` | `users.reset_password` | `{password?}` | 200 `{user, temporary_password, temporary_password_expires_at}` |
| `POST /users/{id}/unlock` | `users.unlock` | — | 200 `UserDetail`（幂等） |
| `PUT /users/{id}/roles` | `users.assign_role` | `{role_ids: uuid[]}` | 200 `UserDetail`；基线 `user` 始终保留 |
| `DELETE /users/{id}/sessions` | `sessions.revoke` | — | 204；递增 `token_version` |

- 列表默认 `-created_at`，排序白名单 `username`/`email`/`display_name`/`status`/`created_at`/`last_login_at`/`updated_at`；`q` 对 username/email/display_name 做大小写不敏感子串搜索；`role_id` 匹配持有该角色的用户（含基线 `user`）；`is_locked` 按 `locked_until > now()` 派生；`must_change_password` 精确匹配。
- `PATCH /users/{id}` 空对象 422 `empty_update`；username/email 冲突 409 `identifier_taken`；修改 username/email 不撤销会话，写审计。
- 创建/重置统一签发临时密码：省略 `password` 则生成 24 字符 base64url；手输必须通过密码策略；统一 `must_change_password=true`、默认 72h 过期；响应只返回一次；管理员不能对自己重置；重置会撤销目标用户全部会话、递增 `token_version`、清除账号层锁定。
- 角色分配整体替换：基线 `user` 不可移除；`admin`/`super_admin` 仅超级管理员可授予/撤销；目标权限集必须是操作者有效权限子集，否则 403 `permission_subset_violation`；未知/软删除角色 422 `unknown_role`。

**Roles**：

| 端点 | 权限 | 请求要点 | 成功 |
|---|---|---|---|
| `GET /roles` | `roles.read` | `q`、`is_builtin`、`page`、`page_size≤200`、`sort` | 200 列表 `RoleSummary` |
| `GET /roles/{id}` | `roles.read` | — | 200 `RoleDetail`；软删除/不存在 404 |
| `POST /roles` | `roles.create` | `{name, description?, permission_keys}` | 201 + Location `RoleDetail` |
| `PATCH /roles/{id}` | `roles.update` | `{name?, description?}` | 200 `RoleDetail` |
| `PUT /roles/{id}/permissions` | `roles.update` | `{permission_keys}` | 200 `RoleDetail` |
| `DELETE /roles/{id}` | `roles.delete` | — | 204（软删除） |

- 内置角色任何写操作 403 `role_immutable`；自定义角色仅当目标权限集是操作者有效权限子集时可写，否则 403 `permission_subset_violation`；删除时仍有分配（含停用用户）409 `role_in_use`；名称冲突 409 `role_name_taken`；空权限 422 `empty_permissions`；非法权限 422 `invalid_permission`。
- 列表默认 `name asc`；`q` 搜 name/description；软删除角色从列表隐藏、名称可复用；列表不按管理范围隐藏，返回 `assignable`/`manageable` 标志。

**Permissions**：

- `GET /permissions` 需要 `roles.read`、`roles.create`、`roles.update` 中任意一项；无分页；返回 `{groups: PermissionGroup[]}`，按代码目录固定顺序。
- `grantable` 对超级管理员恒 true，其他人按「该权限是否在自身有效权限内」计算；目录以代码为权威，数据库 `permissions` 表只做镜像与 FK。

**Audit**：

- `GET /audit-logs` 需要 `audit.read`；过滤 `actor_id`、`action`、`target_type`、`target_id`、`result`、`request_id`、`from`、`to`；默认排序 `-occurred_at`；返回 `AuditLogSummary` 列表。
- `GET /audit-logs/{id}` 返回 `AuditLogDetail`；404 `not_found`。
- `from` 含、`to` 不含；未提供时默认最近 30 天，最大跨度 90 天，超限 422 `time_range_too_large`，格式错误 422 `invalid_time_range`。
- 审计日志只读、不按管理范围过滤；CLI 事件 `actor.id=null`、`actor.label="cli"`；`details` 为开放 object，具体事件字段由「安全基线：密码、限流、审计事件」定义。

### 8. 数据模型

所有主键 `uuid PRIMARY KEY DEFAULT uuidv7()`（PG18-only，不做 PG17 回退）；所有时间为 `timestamptz`；`created_at`/`updated_at` 应用显式维护，不用触发器；外键一律 `ON DELETE RESTRICT`；仅自定义角色软删除，users 永不删除，会话用 `revoked_at`。

- `users`：`id`、`username`、`email`、`display_name`、`phone`、`password_hash`、`status text CHECK (active|disabled)`、`must_change_password bool`、`temporary_password_expires_at`、`token_version bigint`、`failed_login_count int`、`locked_until`、`last_login_at`、`created_at`、`updated_at`。username/email 存规范化小写，普通 `UNIQUE` + `CHECK (col = lower(col))` 兜底；`pg_trgm` GIN 索引支持 `q` 子串搜索。
- `roles`：`id`、`name`、`description`、`is_builtin bool`、`created_at`、`updated_at`、`deleted_at`；部分唯一索引 `lower(name) WHERE deleted_at IS NULL`；内置角色 `user`/`admin`/`super_admin` 由代码定义。
- `permissions`：`key text PRIMARY KEY`、`group_key`、`display_name_zh`、`display_name_en`；代码目录镜像，只在迁移/启动时同步。
- `role_permissions`：`role_id`、`permission_key`，复合主键；外键 RESTRICT。
- `user_roles`：`user_id`、`role_id`、`assigned_at`、`assigned_by`（nullable），复合主键；基线 `user` 显式落一行。
- `auth_sessions`：`id uuid`（即 access token 的 `sid`）、`user_id`、`created_at`、`last_seen_at`、`idle_expires_at`、`absolute_expires_at`、`revoked_at`、`revoke_reason`、`ip inet`、`user_agent text`；索引 `(user_id, last_seen_at)` 用于会话上限淘汰。
- `auth_refresh_tokens`：`token_hash bytea PRIMARY KEY`（SHA-256）、`session_id`、`rotation_counter bigint`、`issued_at`、`expires_at`、`used_at`、`replaced_by_hash bytea UNIQUE`（nullable）、`revoked_at`、`revoked_reason`；索引 `session_id`。
- `auth_login_failures`：`key_hash bytea PRIMARY KEY`（未知标识的 HMAC）、`failed_count int`、`locked_until`、`updated_at`；已知用户使用 `users` 上的计数列。
- `audit_logs`：`id`、`occurred_at`、`actor_user_id`（nullable）、`actor_label`、`action`、`target_type`、`target_id`、`result`、`ip`、`user_agent`、`request_id`、`details jsonb`；**不加外键**；索引 `occurred_at desc`、`actor_user_id`、`action`、`target_id`；应用角色无 UPDATE/DELETE 权限，v1 不分区。
- 刷新令牌 grace：`new_raw = base64url(HMAC-SHA256(refresh_derivation_key, old_raw || ":" || rotation_counter))`；数据库只存 SHA-256；重试时用请求中的旧令牌与 `rotation_counter` 沿链现算后继并比对哈希，返回当前有效令牌；派生密钥独立于 JWT 签名密钥，轮换时旧密钥至少保留 grace 窗口；客户端必须单飞刷新。
- 会话上限 5：第 6 次登录自动撤销最旧的活跃会话（按 `last_seen_at`）并写审计 `authn_session_evicted`；`last_seen_at`/`idle_expires_at` 最多每 60 秒更新一次，idle 超时检查每请求执行。
- 清理任务：过期/撤销的 refresh token、过期的 `auth_login_failures` 行由运维任务删除；具体保留期由部署/安全基线票据确定。

### 9. 迁移、目录同步与种子

- 迁移文件用 UTC 时间戳前缀，`sqlx::migrate!()` 内嵌；生产**不在应用启动时自动迁移**，迁移作为独立部署步骤、用 DDL 权限的 migration role 执行；runtime role 无 DDL 权限。`.sqlx` offline 元数据提交；CI 用 PG18 service 跑 `cargo sqlx prepare --check --workspace -- --all-targets --all-features`；构建用 `SQLX_OFFLINE=true`。`.gitattributes` 固定 `*.sql text eol=lf`。
- 权限目录与内置角色权限集在**启动时**从代码同步：事务内加 advisory lock，upsert `permissions` 与 `role_permissions`；v1 不删除目录中不存在的旧项；内置 `user`/`admin`/`super_admin` 角色由启动同步创建。
- 生产不 seed 任何用户；首个超管只走 CLI 引导；开发/测试 fixture 与生产种子严格分离。
- PG18 作为硬下限：使用 `uuidv7()` 与 `pg_trgm`；不承诺 PG17 回退；不使用 PG18 `oauth` HBA；生产用 SCRAM + `sslmode=verify-full`。

### 10. v1 边界与跨票据边界

- 本契约不包含：批量启用/停用、会话列表/单条撤销（仅全部撤销）、角色恢复、审计导出、用户删除、模拟登录、API Key/OAuth、邀请/邮件流程、管理员永久密码。
- 「安全基线：密码、限流、审计事件」决定：Argon2id 参数与升级、密码策略细节、账号/IP/全局限流阈值、step-up 与重新认证、审计事件清单/级别/保留期、不可篡改存储、安全响应头/CORS/CSP、密钥管理。若引入 step-up，需增量定义 `auth_time` 与重新认证端点并修订 ADR-0001/本 ADR。
- 「部署、可观测与 CI」决定：实际域名/证书/网关、健康检查与 metrics、CI、容量/SLO、审计日志保留与清理。
- 「前端认证集成与路由守卫」「前端裁剪与中英 i18n」「Users 页面真实 CRUD 集成」「角色管理 API 与 UI」「审计日志查询 UI」决定 UI 细节与错误码本地化映射；API 契约以本 ADR 为准。

## Consequences

- **正面**：前后端共享一份稳定、可生成的契约；错误码可本地化且可测试；offset 分页与显式筛选易于实现；UUIDv7 + timestamptz + 规范化标识让索引与审计稳定；派生式 refresh 在只存哈希的前提下满足 grace 幂等；管理范围覆盖读取，避免管理员看到无权管理的用户。
- **负面**：每请求仍有一次数据库授权/会话查询；offset 深分页在数据量大时变慢；派生式 refresh 要求客户端单飞刷新并增加密钥轮换约束；`pg_trgm` 与 `uuidv7()` 绑定 PG18；拒绝未知字段会让新旧客户端部署漂移更早暴露。
- **复查条件**：审计/用户量导致 offset 分页不可用时引入 keyset；刷新派生密钥轮换频繁时评估加密 grace 方案；出现批量操作/导出需求时新增专用端点；权限缓存需求出现时按 ADR-0002 复查；step-up 决策落地时修订认证契约。

## Considered Options

- **cursor/keyset 分页**：深分页更稳，但与 users 页面的页码/总数交互不匹配；v1 不选，留作复查条件。
- **`page_size` 上限 1000**：降低批量拉取调用次数，但放大响应体、内存与滥用成本；v1 不选，改用专用导出。
- **加密 grace 令牌存库**：实现简单，但需要在 grace 窗口保存可解密原文并引入密钥/清理路径；派生式方案保留「只存哈希」；不选。
- **放弃 grace、重放即撤销整族**：更严格，但网络重试会导致误登出；不选。
- **服务端按 Accept-Language 返回本地化错误**：客户端简单，但服务端需要维护文案、测试矩阵扩大；v1 不选。
- **`/admin` 路径分类**：URL 与权限模型重复；不选。
- **角色/权限列表对非超管隐藏**：UI 简单但会出现「列表里不存在的角色仍挂在用户身上」；不选，改为全量 + 标志。
- **users 读取不受管理范围约束**：实现简单但违反最小权限；不选。
- **乐观并发（version/ETag）**：减少丢失更新，但增加客户端复杂度；v1 不选，last-write-wins + 审计追溯。

## References

- [API 与数据契约](https://github.com/nasa-du/rs-admin-ds/issues/8)
- [ADR-0001: 认证令牌与会话模型](./0001-auth-token-session-model.md)
- [ADR-0002: RBAC 权限模型与授权规则](./0002-rbac-permission-model.md)
- [ADR-0003: 用户生命周期与凭据管理](./0003-user-lifecycle-and-credential-management.md)
- `docs/research/axum-stack.md`
- `docs/research/auth-security.md`
- RFC 7807（Problem Details）、RFC 3339（时间格式）、RFC 9562（UUIDv7）、OpenAPI 3.1
