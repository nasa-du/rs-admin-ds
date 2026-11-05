# ADR-0001: 认证令牌与会话模型

- Status: accepted
- Date: 2026-11-05

## Context

前端与 API 分离部署，生产域名尚未确定；本规格按同站子域（`admin.example.com` + `api.example.com`）编写，实际域名、证书与网关归属由部署票据收集。系统不引入 Redis，使用 PostgreSQL 18 保存会话、刷新令牌与撤销状态；JWT 是既定认证方式。管理后台需要账号停用、改密与全部登出后的即时失效，并且必须支持多实例部署。

## Decision

1. **部署拓扑**：默认同站不同源。refresh cookie 是第一方 cookie；如果实际域名无法同站，按本 ADR 的回退条件改用 BFF，不把第三方 cookie 作为生产主方案。
2. **访问令牌**：JWT，EdDSA（Ed25519），header 带 `kid`，TTL 15 分钟。Claims 只包含 `sub`、`sid`、`ver`、`iss`、`aud`、`exp`、`iat`、`nbf`、`jti`、`typ=at+jwt`；只存 JavaScript 内存，通过 `Authorization: Bearer` 发送；不承载角色或权限。
3. **刷新令牌**：256-bit 不透明随机串，数据库只存 SHA-256。通过 `__Host-refresh` Cookie 传输，属性为 `HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=43200`，不设 `Domain`。每次刷新轮换，按会话形成令牌族；30 秒 grace 内幂等重试，grace 外重放撤销整族。
4. **会话**：服务端记录 idle 30 分钟、absolute 12 小时；每用户最多 5 个活跃会话。
5. **撤销**：每个受保护请求按 `sid` 校验会话状态，并比对 `users.token_version`；数据库不可用时 fail-closed；v1 不做缓存。登出撤销当前会话；全部登出、管理员重置凭据、停用用户撤销全部会话并递增 `token_version`。自助改密保留当前会话：校验当前密码后撤销其他全部会话、递增 `token_version`，并为当前会话重签 access token（见 ADR-0003）。
6. **CSRF/CORS**：精确 Origin allowlist 加 credentials；登录、刷新、登出要求 POST + `application/json` + `X-Requested-With`；业务 API 只用 Bearer，不读取 cookie；不做 double-submit cookie。
7. **多实例**：刷新事务使用 `SELECT ... FOR UPDATE`；不使用进程内存保存会话或限流状态；JWT 密钥从 secret manager 加载，按 `kid` 轮换，旧公钥至少保留 1 小时。

## Amendments

- 2026-11-05：§5 的自助改密语义由 ADR-0003 修订——自助改密保留当前会话，撤销其他会话并递增 `token_version`，为当前会话重签 access token；管理员重置凭据与停用用户仍撤销全部会话并递增 `token_version`。v1 不提供用户删除。
- 2026-11-05：§3 的 grace 实现明确为派生式刷新令牌——`new_raw = base64url(HMAC-SHA256(refresh_derivation_key, old_raw || ":" || rotation_counter))`；数据库仍只存 SHA-256；`auth_refresh_tokens` 记录 `rotation_counter` 与 `replaced_by_hash`；重试时沿链现算后继并返回当前有效令牌；派生密钥独立于 JWT 签名密钥，轮换时旧密钥至少保留 grace 窗口；客户端必须单飞刷新（见 ADR-0004）。
- 2026-11-05：§4 的「每用户最多 5 个活跃会话」明确为第 6 次登录自动撤销最旧会话（按 `last_seen_at`）并写审计 `authn_session_evicted`；`last_seen_at` 与 idle 截止时间最多每 60 秒更新一次。
- 2026-11-05：§5 补充管理员会话撤销——`DELETE /users/{id}/sessions` 撤销目标用户全部会话并递增 `token_version`；管理员不得对自己执行该操作（走 `/auth/logout-all`）。自助改密在保留当前会话的同时轮换其 refresh token（见 ADR-0004）。

## Consequences

- **正面**：同站子域避免第三方 cookie 拦截；`sid` + `ver` 让停用、改密、登出即时生效；角色与权限不进入 JWT，RBAC 变更无需等待令牌过期；不依赖 Redis。
- **负面**：每个受保护请求增加一次数据库授权查询；PostgreSQL 成为认证关键路径，故障时认证 fail-closed；12 小时持久 refresh cookie 是对 NIST「bearer secret 不应跨浏览器重启持久化」的有意偏离，需要在安全评审中记录。
- **复查条件**：实际域名无法同站 → 重新评估 BFF；数据库压力需要缓存 → 引入 5–15 秒缓存并文档化撤销窗口；出现第二个验签服务 → 发布 JWKS。

## Considered Options

- **HS256**：更简单，但未来多验签方需要分发共享密钥；不选。
- **把 roles/permissions 放进 JWT**：减少查询，但角色变更要等 token 过期或强制刷新；不选。
- **refresh token 用 JWT**：无法可靠撤销与 reuse detection；不选。
- **access token 放 localStorage 或 JS 可读 cookie**：XSS 可直接窃取；不选。
- **只靠短 TTL 撤销**：停用账号最多 15 分钟后仍有效；不选。
- **真跨站 + `SameSite=None`/CHIPS**：Safari ITP 与 Chrome 无痕不可靠；仅作为无同站域名时的过渡，长期用 BFF。

## References

- [跨域认证与令牌/会话模型](https://github.com/nasa-du/rs-admin-ds/issues/5)
- `docs/research/auth-security.md`
- `docs/research/axum-stack.md`
- [ADR-0003: 用户生命周期与凭据管理](./0003-user-lifecycle-and-credential-management.md)
