# 认证安全最佳实践调研（rs-admin-ds）

> 调研日期：2026-11-05
>
> 适用范围：React SPA（shadcn-admin fork）+ Rust Axum API + SQLx + PostgreSQL 18；JWT access token；独立前端/API 部署。
>
> 文档性质：研究结论与实施建议，不是生产代码。本文中的阈值（TTL、锁定次数、保留期）是建议基线，落地前需经产品、安全和合规评审，并在生产硬件上压测。
>
> 版本基线：
> - OWASP ASVS **5.0.0**（2025-05-30 稳定版）；GitHub 上另有 2026-10-26 的 `latest` bleeding-edge 分支，本文不引用其未稳定条目。
> - OWASP Cheat Sheets：以 2026-11-05 在线版本为准。
> - NIST SP 800-63B-4（2025-07 final）。
> - RFC 9700（OAuth 2.0 Security BCP，2025-01）、RFC 9106、RFC 7519、RFC 6750、RFC 8725、RFC 9068、RFC 7517、RFC 7515。
> - RFC 6265bis `draft-ietf-httpbis-rfc6265bis-22`（2025-12；已获 IESG 批准为 Proposed Standard，截至 2026 年仍在 RFC Editor 队列中，尚未分配 RFC 号）。
> - 浏览器：WebKit/Safari ITP、Chrome 第三方 Cookie 文档（2026-11 现状）。

---

## 0. 结论速览

| 决策点 | 推荐结论 | 核心理由 |
|---|---|---|
| 前端/API 部署 | 优先同一 registrable domain（如 `app.example.com` + `api.example.com`）；做不到则用同源 BFF | 同 site 下 refresh cookie 不是 third-party cookie，Safari ITP/Chrome Incognito 不会拦截；跨 site 的 third-party cookie 不可靠 |
| Access token | JWT，ES256 或 EdDSA，TTL 10–15 分钟，只放 JavaScript 内存，通过 `Authorization: Bearer` 发送 | Bearer token 短寿命可限制泄漏影响；放 header 不自动携带，天然规避大部分 CSRF |
| Refresh token | 不透明随机串（256-bit），`HttpOnly; Secure; SameSite=Strict/Lax` Cookie；每次刷新轮换 + reuse detection | 可服务端撤销；HttpOnly 防 XSS 读取；轮换可检测重放 |
| Access token 撤销 | 短 TTL + `sid` 会话校验 + 每用户 `token_version`；必要时加 `jti` denylist | 纯 stateless JWT 无法单独撤销；管理后台需要“退出即失效/改密即失效” |
| CSRF | API 用 header token 时不依赖 Cookie；refresh/logout 用 `SameSite` + 精确 `Origin` allowlist + 自定义 header/CSRF token | CORS 本身不是 CSRF 防御；Cookie 自动携带才是 CSRF 根因 |
| CORS | 精确 origin allowlist；`Access-Control-Allow-Credentials: true`；禁止 `*` 配 credentials；`Vary: Origin` | Fetch 规范明确 credentials 模式下 `*` 无效 |
| 密码哈希 | Argon2id；建议 `m=64 MiB, t=3, p=4`，不低于 OWASP `m=19 MiB, t=2, p=1`；PHC 字符串存储参数 | 内存硬 + 可调成本；参数需按生产硬件 benchmark |
| 密码策略 | 单因子最短 15 字符；有 MFA 最短 8；最长至少 64（建议 128）；blocklist；禁止组合规则和定期轮换 | NIST 800-63B-4 与 ASVS 5.0 的明确要求 |
| 登录限速 | PostgreSQL 原子 upsert；account + IP + global 三层；指数退避/临时锁定；CAPTCHA/MFA 升级；不做永久自动锁定 | 无 Redis 时数据库是共享权威状态；per-IP 单独不够，account 层是主防线 |
| 审计 | OWASP Logging Vocabulary 命名；UTC 结构化 JSON；不记录 secret；保留期按策略（示例：热 90 天、温 1 年、冷 3 年） | ASVS V16 + NIST AU-11/800-92；便于检测和取证 |
| JWT 密钥轮换 | 非对称密钥 + `kid` + JWKS/公钥集合；先发布新公钥、再切换签名、旧公钥至少保留一个 token 最大寿命；准备应急轮换 | 避免轮换造成全量掉线；私钥泄漏时必须 fail-closed |

---

## 1. 前后端分离下的跨域令牌传输

### 1.1 先分清 origin、site 和 cookie 的“第一方/第三方”

- **Origin** 由 scheme + host + port 组成；三者任一不同就是 cross-origin。`https://app.example.com` 与 `https://api.example.com` 是 cross-origin；`https://app.example.com:443` 与 `https://app.example.com:8443` 也是 cross-origin。参见 [WHATWG HTML](https://html.spec.whatwg.org/multipage/browsers.html#same-site)。
- **Site** 由 scheme + registrable domain（eTLD+1）决定；**port 被忽略**。`app.example.com` 与 `api.example.com` 的 registrable domain 都是 `example.com`，因此是 **same-site**，但仍是 cross-origin。参见 [WHATWG HTML](https://html.spec.whatwg.org/multipage/browsers.html#same-site)。
- Cookie 的 `SameSite` 判断基于 site，不是 origin；`SameSite=Lax/Strict` 的 Cookie 会随 same-site 的跨 origin 请求发送。参见 [RFC 6265bis](https://www.ietf.org/archive/id/draft-ietf-httpbis-rfc6265bis-22.html) 与 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- WebKit 对 first-party/third-party 的定义同样是“不同网站（site）”：`sub.news.example` 在 `news.example` 下被视为 first-party。参见 [WebKit Tracking Prevention](https://webkit.org/tracking-prevention/)。
- 因此，**same-site 子域部署不会触发 third-party cookie 拦截**；但它是 cross-origin，仍然需要正确的 CORS 配置。**CORS 与 SameSite 解决的是不同问题**。

### 1.2 三种部署拓扑

| 拓扑 | 示例 | Cookie 属性 | 现实可行性 | 结论 |
|---|---|---|---|---|
| 同源（推荐之一） | `https://admin.example.com` 同时提供 SPA 和 `/api` | first-party；`SameSite=Strict` 可用 | 最简单；无 CORS；无 third-party cookie 问题 | 若部署允许，优先 |
| 同 site、跨 origin（推荐） | `https://app.example.com` + `https://api.example.com` | first-party（same-site）；`SameSite=Strict/Lax` | 需要 CORS + credentials；Safari ITP 不拦截 | 前后端独立部署时首选 |
| 真正 cross-site（不推荐） | `https://app.vercel.app` + `https://api.example.com` | third-party；必须 `SameSite=None; Secure` | Safari ITP 默认阻断；Chrome Incognito 默认阻断；Chrome 普通模式用户可阻止 | 仅作过渡；优先改 BFF |
| cross-site + CHIPS | 同上，加 `Partitioned` | `SameSite=None; Secure; Partitioned` | Chromium 支持，Safari 不支持；按 top-level site 分区 | 仅 Chromium 的有限缓解，不是跨浏览器方案 |

浏览器现状（2026-11）：

- Safari/WebKit ITP **默认阻止所有 third-party cookie**；“There are no exceptions to this blocking”，只能通过 Storage Access API 或临时 popup 兼容机制获得访问。参见 [WebKit Tracking Prevention](https://webkit.org/tracking-prevention/)。
- Chrome **Incognito 默认阻止 third-party cookie**；普通模式由用户设置允许/阻止，并可按站点例外。参见 [Chrome Help: Cookies](https://support.google.com/chrome/answer/95647)。
- Google 2025-10-17 的 Privacy Sandbox 更新明确 Chrome 维持“给用户第三方 Cookie 选择”的现有路线，而不是无条件全量禁用。参见 [Privacy Sandbox Update, 2025-10-17](https://privacysandbox.google.com/blog/update-on-plans-for-privacy-sandbox-technologies)。
- CHIPS（`Partitioned`）让 third-party cookie 按 top-level site 分区，是 Chromium 在 third-party cookie 被阻止时仍可读写 cookie 的机制之一；但 Safari 没有等价的普遍支持。参见 [Chrome CHIPS](https://privacysandbox.google.com/cookies/chips)。

**结论：不要把“跨 site 的 refresh cookie”作为生产架构的唯一基础。** 同一 registrable domain 的同 site 部署，或同源 BFF，才是可跨浏览器稳定工作的方案。

### 1.3 推荐方案 A：同 site 子域 + 内存 access token + HttpOnly refresh cookie

这是“前后端独立部署”场景下最平衡的方案。

**令牌放置**

| 令牌 | 放置位置 | 传输方式 | 说明 |
|---|---|---|---|
| Access token | JavaScript 内存（变量/React context） | `Authorization: Bearer <jwt>` | 页面刷新后丢失，启动时用 refresh 恢复；不写 `localStorage`/`sessionStorage` |
| Refresh token | `HttpOnly` Cookie，由 API 域设置 | 浏览器自动携带到 refresh 端点 | JS 不可读；只在 `/auth/refresh`、`/auth/logout` 使用 |

**Refresh Cookie 建议**

```http
Set-Cookie: __Host-refresh=<opaque-256-bit>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=43200
```

- `HttpOnly`：禁止 JavaScript 读取，降低 XSS 直接窃取 refresh token 的风险。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- `Secure`：只经 HTTPS 发送；session cookie 必须使用。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- `SameSite=Strict`（或 `Lax`）：CSRF 纵深防御，不是唯一防线。OWASP 明确要求 session cookie 显式设置 `Strict`（优先）或 `Lax`，不得依赖浏览器默认值；`None` 必须配 `Secure`。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- `__Host-` 前缀：要求 `Secure`、无 `Domain`、`Path=/`，可防止子域伪造和 HTTPS 降级。参见 [RFC 6265bis §4.1.3.2](https://www.ietf.org/archive/id/draft-ietf-httpbis-rfc6265bis-22.html) 与 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- **`__Host-` 与窄 Path 的冲突**：`__Host-` 强制 `Path=/`，不能把 refresh cookie 限定到 `/api/auth/refresh`。两种选择：
  1. **推荐**：`__Host-refresh` + `Path=/`，服务端只在 refresh/logout 端点读取它，其他端点忽略；同时禁止记录 `Cookie` header。安全性优先。
  2. 若必须窄 Path：使用 `__Secure-refresh` + `Path=/api/auth`（仍不设 `Domain`）。但 `__Secure-` 不阻止同级子域设置同名 `Domain` cookie，防护弱于 `__Host-`；必须配合服务端多 cookie 解析策略和子域安全治理。
- **持久化取舍**：NIST SP 800-63B-4 §5.1.1 指出，作为 bearer secret 的 session secret **SHOULD NOT** 跨浏览器重启持久化；设置 `Max-Age` 的 refresh cookie 属于持久化。管理后台为 UX 使用 12 小时持久 refresh cookie 是可接受的 deviation，但必须文档化、强制 idle/absolute timeout、并保持轮换 + reuse detection；更高安全级别应省略 `Max-Age`（session cookie）并缩短 absolute timeout。参见 [NIST SP 800-63B-4 §5.1.1](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- **不要设置 `Domain=.example.com`**：这会让所有子域共享/可覆盖 cookie，扩大攻击面。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html) 与 [ASVS V3 3.3.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x12-V3-Web-Frontend-Security.md)。

**前端请求**

```ts
// 业务 API：只带 Authorization，不带 cookie
fetch("https://api.example.com/v1/users", {
  headers: { Authorization: `Bearer ${accessToken}` },
});

// refresh/logout：需要 cookie；用自定义 header 触发 preflight
fetch("https://api.example.com/auth/refresh", {
  method: "POST",
  credentials: "include",
  headers: { "X-Requested-With": "XMLHttpRequest", "Content-Type": "application/json" },
  body: "{}",
});
```

- 业务 API 不使用 cookie，因此 CSRF 不适用于这些端点（攻击者无法跨站设置 `Authorization` header）。
- refresh/logout 使用 cookie，必须做 CSRF 防护（见第 4 节）。
- **冷启动的 CSRF token 问题**：登录/refresh 响应可以返回与会话绑定的 CSRF token 并保存在内存，但页面刷新后内存清空，第一次 refresh 无法带上该 token。因此 refresh 端点应依赖“精确 Origin allowlist + 必须存在的自定义 header + SameSite”作为主防线；会话级 CSRF token 只用于登录后的状态变更请求。若坚持每次 refresh 都校验随机 CSRF token，则需把 token 放在 JS 可读的 first-party cookie（同源/BFF 可行，跨子域不可行）或改用 BFF。
- CORS 必须允许 `https://app.example.com` 且 `Access-Control-Allow-Credentials: true`；不能使用 `*`。参见 [WHATWG Fetch §3.3.5](https://fetch.spec.whatwg.org/#cors-protocol-and-credentials)。

**启动恢复流程**

1. SPA 加载，access token 为空。
2. 调用 `POST /auth/refresh`（带 refresh cookie）。成功则拿到新 access token（以及轮换后的 refresh cookie）。
3. 失败（401）则跳转登录页。
4. 之后的业务请求只带 access token；access token 过期时，单飞（single-flight）调用 refresh，避免并发刷新。

### 1.4 推荐方案 B：同源 BFF（真正跨 site 或高安全场景）

如果前端必须部署在无法与 API 共享 registrable domain 的平台（例如 `*.vercel.app`），推荐增加一个 **Backend-for-Frontend**：

- 浏览器只与 BFF 同源通信；BFF 设置 first-party session cookie（`HttpOnly; Secure; SameSite=Strict`）。
- Refresh token/access token 保存在 BFF 服务端（内存/数据库/密钥库），浏览器完全不接触 refresh token。
- BFF 以服务端身份调用内部 API；API 可以只暴露在内网，使用 mTLS 或服务账号。
- 这样没有 third-party cookie、没有浏览器 CORS credentials 复杂性，XSS 也无法直接读取 refresh token。

OWASP Session Management 在“不要用 localStorage 存 token”的段落中明确给出替代方案：`HttpOnly; Secure; SameSite=Strict` cookie 或 **Backend-for-Frontend (BFF)**。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。

**BFF 的代价**：多一个组件、需要处理会话存储和高可用；但对“跨 site + 管理后台”来说，通常比依赖 third-party cookie 更可靠。

### 1.5 如果必须真正 cross-site：最低要求与限制

如果短期无法改部署，只能使用 `SameSite=None`：

```http
Set-Cookie: __Host-refresh=<opaque>; Path=/; Secure; HttpOnly; SameSite=None; Partitioned; Max-Age=43200
```

限制与注意事项：

- `SameSite=None` **必须**同时有 `Secure`，否则浏览器拒绝设置。参见 [RFC 6265bis](https://www.ietf.org/archive/id/draft-ietf-httpbis-rfc6265bis-22.html) 与 [Chrome SameSite 文档](https://web.dev/articles/samesite-cookies-explained)。
- Safari ITP 仍会阻止 third-party cookie；CHIPS 不是 Safari 的通用解决方案。参见 [WebKit Tracking Prevention](https://webkit.org/tracking-prevention/)。
- Chrome Incognito 默认阻止；普通模式用户可在设置中阻止。参见 [Chrome Help: Cookies](https://support.google.com/chrome/answer/95647)。
- `Partitioned` 会按 top-level site 分区；同一用户在不同 top-level site 下是不同会话，且并非所有浏览器支持。参见 [Chrome CHIPS](https://privacysandbox.google.com/cookies/chips)。
- 必须准备降级方案：检测 refresh 失败/ Cookie 不可用，提示用户使用同源入口或 BFF；不要静默把 refresh token 改存 `localStorage`。

### 1.6 Cookie 属性速查

| 属性/前缀 | 作用 | 推荐 |
|---|---|---|
| `Secure` | 只经 HTTPS 发送 | 必须 |
| `HttpOnly` | JS 不可读 | refresh cookie 必须 |
| `SameSite=Strict` | 仅 same-site 请求携带 | 首选（无跨 site 顶层流程时） |
| `SameSite=Lax` | same-site + 跨 site 顶层安全导航 | 兼容性备选 |
| `SameSite=None` | 跨 site 携带 | 仅真正 cross-site，必须 `Secure` |
| `Path=/` | 全站路径 | `__Host-` 要求；服务端按端点限制用途 |
| `Path=/api/auth` | 窄路径 | 仅 `__Secure-` 可用；防护弱于 `__Host-` |
| `Domain` | 共享到子域 | **省略**（host-only） |
| `Max-Age`/`Expires` | 浏览器持久化时间 | 设为会话绝对上限；服务端仍必须独立校验 |
| `__Host-` | Secure + 无 Domain + Path=/ | refresh cookie 首选 |
| `__Secure-` | 仅要求 Secure | 需要窄 Path 时的退让方案 |
| `Partitioned` | CHIPS 分区 | 仅 Chromium 过渡方案，不作跨浏览器依赖 |

Cookie 过期时间不是安全边界：浏览器可能保留 session cookie 或用户可修改本地状态；服务端必须用 `auth_sessions.absolute_expires_at` / `idle_expires_at` 强制执行。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html) 与 [NIST SP 800-63B-4 §5.1.1](https://pages.nist.gov/800-63-4/sp800-63b.html)。

### 1.7 反模式

- ❌ 把 access token 或 refresh token 放进 `localStorage`/`sessionStorage`。任何 XSS 都能读取全部 token；OWASP 明确禁止。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- ❌ 把 refresh token 放在 URL、query、fragment 或日志中；会进入浏览器历史、Referer、代理日志。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- ❌ `SameSite=None` 不带 `Secure`；浏览器会拒绝。参见 [RFC 6265bis](https://www.ietf.org/archive/id/draft-ietf-httpbis-rfc6265bis-22.html)。
- ❌ 设置 `Domain=.example.com`，让所有子域共享 refresh cookie。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- ❌ 只删除浏览器 cookie 就算 logout；服务端 refresh token 仍然有效。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- ❌ 把 refresh token 当作长期 bearer token，不轮换、不检测 reuse、不做服务端会话记录。
- ❌ 依赖 third-party cookie 作为唯一认证通道，却不测试 Safari/Chrome Incognito。
- ❌ CORS 使用 `Access-Control-Allow-Origin: *` 且 `Allow-Credentials: true`；浏览器不会接受，且是危险配置。参见 [WHATWG Fetch](https://fetch.spec.whatwg.org/#cors-protocol-and-credentials)。
- ❌ 在 Cookie 认证的 API 上只依赖 CORS 防 CSRF；CORS 只限制读取响应，不阻止请求发送。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。

### 1.8 部署决策清单

1. 能否让 SPA 与 API 同源？→ 同源，最简单、最稳。
2. 能否让二者共享 registrable domain（`app.example.com` + `api.example.com`）？→ 同 site 子域，推荐。
3. 二者必须跨 site？→ 上 BFF；不要以 third-party cookie 为主方案。
4. 只能跨 site 且无 BFF？→ 接受 Safari/Incognito 不可用；用 `SameSite=None; Secure; Partitioned` 仅作 Chromium 过渡，并准备降级提示。
5. 无论哪种拓扑：HTTPS + HSTS、`__Host-`、无 `Domain`、精确 CORS、refresh 轮换 + reuse detection。

---

## 2. Access JWT：claims、TTL、签名与撤销语义

### 2.1 推荐 claims

| Claim/Header | 必需性 | 值/示例 | 验证要求 |
|---|---|---|---|
| `alg` (header) | 必须 | `ES256` 或 `EdDSA` | 服务端 allowlist；拒绝 `none`、拒绝算法混淆 |
| `typ` (header) | 推荐 | `at+jwt` | 拒绝缺失/不匹配的类型，防止跨 JWT 混淆 |
| `kid` (header) | 必须（多密钥时） | `2026-11-01-a` | 只能从受信任的 JWKS/公钥集合中选 key |
| `iss` | 必须 | `https://api.example.com` | 精确、区分大小写匹配 |
| `aud` | 必须 | `https://api.example.com` 或稳定的 API identifier | 必须包含本服务；不能拿 ID token 当 access token |
| `sub` | 必须 | 稳定 `user_id`（如 UUID/ULID） | 不要用 email/username（会变、含 PII） |
| `exp` | 必须 | NumericDate（秒） | 当前时间必须早于 `exp` |
| `iat` | 必须 | NumericDate | 不应在未来（允许小 skew）；可用于最大寿命检查 |
| `nbf` | 推荐 | NumericDate | 当前时间不得早于 `nbf`（允许小 skew） |
| `jti` | 推荐/必须 | 唯一 ID（UUID/ULID） | 用于 denylist、审计、重放检测 |
| `sid` | 推荐（本系统） | 会话 ID | 与 `auth_sessions.id` 对应，支持单会话撤销 |
| `ver` | 推荐（本系统） | 整数 token version | 与 `users.token_version` 对应，支持全用户撤销 |
| `scope`/`roles` | 可选 | 最小权限集合 | 短 TTL；角色变更需撤销/版本递增，不能永久信任 |
| `auth_time` | 敏感操作推荐 | 最近一次强认证时间 | 敏感操作要求足够新（step-up reauth） |

说明：

- `iss`/`aud`/`exp` 等核心 claim 的语义见 [RFC 7519 §4.1](https://www.rfc-editor.org/rfc/rfc7519.html)。RFC 9068 是 OAuth 2.0 JWT access token profile，要求 `iss`、`exp`、`aud`、`sub`、`client_id`、`iat`、`jti`，并要求 `typ` 为 `at+jwt`；本系统即使不是完整 OAuth，也可以采用这套 claim 约束。参见 [RFC 9068](https://www.rfc-editor.org/rfc/rfc9068.html)。
- 必须验证 `iss` 与 `aud`。只验签名和过期时间，会接受同一密钥签给其他服务/其他用途的 token。参见 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html) 与 [RFC 8725 §3.8/§3.9](https://www.rfc-editor.org/rfc/rfc8725.html)。
- 用 `typ` 和独立密钥区分 access token、ID token、密码重置 token 等，防止 cross-JWT confusion。参见 [RFC 8725 §3.11/§3.12](https://www.rfc-editor.org/rfc/rfc8725.html) 与 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html)。
- `roles` 放 JWT 会让角色变更在 token 过期前不生效。管理后台建议要么短 TTL + 变更时递增 `ver`/撤销会话，要么每次请求从数据库/缓存读取权限；不要无限信任 JWT 里的角色。参见 [ASVS V9 9.2.1](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md) 与 [ASVS V7 7.4.1](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。

### 2.2 TTL 与时钟偏移

| 项目 | 建议值 | 依据 |
|---|---|---|
| Access token TTL | 10–15 分钟（绝对上限 1 小时） | RFC 6750 指出 short-lived（one hour or less）token 可降低泄漏影响；管理后台取更短 |
| Refresh 会话 idle timeout | 30 分钟（管理后台） | NIST AAL2 建议 inactivity ≤ 1 小时；管理后台取更严格 |
| Refresh 会话 absolute timeout | 12 小时（管理后台；不超过 24 小时） | NIST AAL2 overall ≤ 24 小时；管理后台建议 12 小时 |
| Clock skew | 30–60 秒 | RFC 7519 允许少量 leeway；不要用 5 分钟以上 |
| 最大 token 寿命校验 | `exp - iat <= 15 min` | 防止 issuer 被攻破后铸造超长 token；作为纵深防御 |

- RFC 6750 明确建议“short-lived (one hour or less) tokens”以减少泄漏影响。参见 [RFC 6750 §5.3](https://www.rfc-editor.org/rfc/rfc6750.html)。
- NIST SP 800-63B-4 对 AAL2 的 reauthentication 要求是 overall timeout SHOULD ≤ 24 小时、inactivity timeout SHOULD ≤ 1 小时。参见 [NIST SP 800-63B-4 §2.2.3](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- NIST 还明确指出：**access token 不能作为用户“仍在现场”的证明**；access token 和 refresh token 可能在认证会话结束后仍然有效。因此敏感操作（改密、改邮箱、MFA 设置、角色变更、删除用户）不能只看 JWT，必须要求重新认证/step-up。参见 [NIST SP 800-63B-4 §5.1.2](https://pages.nist.gov/800-63-4/sp800-63b.html) 与 [ASVS V7 7.5.1/7.5.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
- 时钟偏移只用于容忍正常 NTP 误差；过大等于放宽过期时间。参见 [RFC 7519 §4.1.4](https://www.rfc-editor.org/rfc/rfc7519.html)。

### 2.3 签名与验证

**算法选择**

- 推荐 **ES256（ECDSA P-256）** 或 **EdDSA（Ed25519）**；二者约 128-bit 安全强度。参见 [ASVS V11 11.2.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x20-V11-Cryptography.md)。
- 不建议让 SPA 持有 HS256 共享密钥；前端代码/构建产物无法保守秘密。多服务验证时，非对称密钥也更易于分发公钥。参见 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html) 与 [RFC 8725 §3.2](https://www.rfc-editor.org/rfc/rfc8725.html)。
- 若必须用 HS256：密钥至少 256-bit 随机值，只存在于服务端；不要与前端共享；不要用人类可记密码。参见 [RFC 8725 §2.2](https://www.rfc-editor.org/rfc/rfc8725.html)。
- 若用 RSA：至少 RSA-3072 才接近 128-bit 安全强度；优先 ES256/EdDSA。参见 [ASVS V11 11.2.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x20-V11-Cryptography.md)。

**验证顺序与规则**

1. 从受信任配置/JWKS 取 key（按 `kid`），不要从 token 里的 `jwk`/`jku`/`x5u` 取 key。参见 [ASVS V9 9.1.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md) 与 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html)。
2. 校验 `alg` 在 allowlist 内；拒绝 `none`；拒绝把公钥当 HMAC 密钥的算法混淆。参见 [ASVS V9 9.1.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md) 与 [RFC 8725 §3.1](https://www.rfc-editor.org/rfc/rfc8725.html)。
3. 验证签名，再解析/使用 claims；签名失败立即拒绝。
4. 验证 `typ`（如 `at+jwt`）、`iss`、`aud`、`exp`、`nbf`、`iat`；缺 required claim 拒绝。参见 [ASVS V9 9.2.1/9.2.2/9.2.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md)。
5. 校验 `sid`/`ver`（如启用）与服务端状态；校验用户未被禁用。
6. 拒绝重复 JSON key、超大 token、未知 critical header；使用成熟 JWT 库，不手写解析。参见 [RFC 7519 §4](https://www.rfc-editor.org/rfc/rfc7519.html) 与 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html)。

### 2.4 撤销语义：refresh 可立即撤销，access 受 TTL 约束

**关键区别**

- **Refresh token 是服务端有状态的**：删除/标记 `auth_sessions` 和 `auth_refresh_tokens` 后，下一次 refresh 立即失败。
- **Access JWT 是自包含的**：签名有效且未过期就会被接受；仅靠 JWT 本身无法单独撤销。ASVS V7 7.4.1 明确说，self-contained token 需要 denylist、按用户的时间点（tokens produced before a per-user date/time）或轮换每用户签名密钥等方案。参见 [ASVS V7 7.4.1](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。

**可选机制**

| 机制 | 粒度 | 成本 | 适用 |
|---|---|---|---|
| 短 TTL | 全部 | 最低 | 所有系统的基础 |
| `sid` 会话校验 | 单会话/设备 | 每请求一次会话查询（可缓存） | 管理后台推荐 |
| `ver` token version | 单用户全部 token | 每请求一次用户查询（可与会话查询合并） | 改密/禁用/角色变更/revoke-all |
| `jti` denylist | 单个 token | 每请求查 denylist + 清理 | 需要精确撤销单个 access token 时 |
| 轮换签名密钥 | 全部 | 运维复杂 | 密钥泄漏/紧急全量失效 |

**推荐组合（本系统）**

1. Access token TTL 10–15 分钟。
2. JWT 带 `sid`；中间件校验 `auth_sessions` 未撤销、未过期。可用进程内缓存 5–15 秒降低 DB 压力；撤销传播延迟 ≤ 缓存 TTL。缓存必须 fail-closed：DB/缓存不可用时拒绝受保护请求，而不是放行。
3. JWT 带 `ver`；中间件校验 `users.token_version`。改密、重置密码、禁用账户、角色变更、revoke-all 时执行 `token_version = token_version + 1`，并撤销该用户所有会话。
4. Logout：撤销当前 `auth_sessions` 行 + 清除 refresh cookie；访问 token 因 `sid` 校验立即失效（若没有 `sid` 校验，则最多等到 TTL 过期）。
5. 只有在需要撤销“某一个 access token 但保留会话”时，才引入 `jti` denylist；按 `(jti, iss)` 存储，到 `exp` 后清理。参见 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html)。
6. 账户禁用/删除必须终止所有会话。参见 [ASVS V7 7.4.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
7. 密码/认证因子变更后，给用户“终止其他会话”的选项，并默认撤销其他会话。参见 [ASVS V7 7.4.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。

**敏感操作 reauth**

- 修改密码、邮箱、MFA 配置、角色/权限、删除用户、查看审计日志等高敏感操作，要求最近一次强认证（例如 5 分钟内）或 step-up MFA；不要仅凭 access token 放行。参见 [NIST SP 800-63B-4 §5.1.2](https://pages.nist.gov/800-63-4/sp800-63b.html) 与 [ASVS V7 7.5.1/7.5.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。

### 2.5 反模式

- ❌ Access token TTL 24 小时/7 天，且没有撤销机制。参见 [RFC 6750](https://www.rfc-editor.org/rfc/rfc6750.html)。
- ❌ 从 token header 读取 `alg` 并直接使用；接受 `alg:none`；HS256 与 RSA 公钥混用。参见 [RFC 8725 §3.1](https://www.rfc-editor.org/rfc/rfc8725.html)。
- ❌ 不验证 `aud`/`iss`；任何同密钥签发的 token 都接受。参见 [RFC 8725 §3.8/§3.9](https://www.rfc-editor.org/rfc/rfc8725.html)。
- ❌ 把 role 永远放在 JWT 里，角色被降权后仍继续有效直到过期。
- ❌ Logout 只删除 cookie，不撤销服务端会话。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- ❌ `jti` denylist 只增不删，导致表无限增长；必须按 `exp` 清理。参见 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html)。
- ❌ 接受 token 里的 `jku`/`jwk`/`x5u` 作为 key 来源；攻击者可指定自己的 key。参见 [ASVS V9 9.1.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md)。
- ❌ access token、ID token、密码重置 token 共用同一密钥且不校验 `typ`；导致跨用途重放。参见 [RFC 8725 §3.11/§3.12](https://www.rfc-editor.org/rfc/rfc8725.html)。
- ❌ Clock skew 设为 5–10 分钟，等于延长 token 寿命。参见 [RFC 7519 §4.1.4](https://www.rfc-editor.org/rfc/rfc7519.html)。

---

## 3. Refresh token：轮换、哈希存储、reuse detection、会话/设备记录与 revoke-all

### 3.1 格式与熵

- 使用 **不透明随机串**，不要用 JWT。refresh token 不需要客户端解析；不透明串更容易做服务端撤销、避免 claims 泄漏。
- 生成：CSPRNG 32 字节（256-bit），base64url 编码；可加 `rt_` 前缀便于识别/日志脱敏，但前缀不携带秘密。
- 最低要求：ASVS 对 reference token 要求至少 128-bit 熵；OWASP 对 session ID 要求至少 64-bit 熵。256-bit 远超要求，推荐。参见 [ASVS V7 7.2.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md) 与 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。

### 3.2 哈希存储

- 数据库只存 `SHA-256(token)`（或 `HMAC-SHA-256(pepper, token)`），不存明文。
- 对 256-bit 随机 token，SHA-256 足够：攻击者无法暴力枚举 2^256 空间；不需要 Argon2。ASVS 6.5.2 明确允许对 ≥112-bit 熵的 secret 使用标准 hash。参见 [ASVS V6 6.5.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- 若加 pepper，pepper 放 KMS/Secrets Manager，不放数据库；pepper 轮换需要重新计算所有 token hash 或让用户重新登录。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)（pepper 原则同样适用于高熵 secret 的额外保护）。
- 不要用 Argon2 哈希 refresh token：每次刷新都做内存硬哈希会增加延迟和 DoS 面；高熵随机 token 不需要慢哈希。
- 查询时按 `token_hash` 唯一索引查找；不要在应用中用非常量时间比较明文 token。

### 3.3 推荐表结构（PostgreSQL）

```sql
-- 一个登录会话/设备一行
CREATE TABLE auth_sessions (
    id                  uuid PRIMARY KEY,
    user_id             uuid NOT NULL REFERENCES users(id),
    created_at          timestamptz NOT NULL DEFAULT now(),
    last_seen_at        timestamptz NOT NULL DEFAULT now(),
    absolute_expires_at timestamptz NOT NULL,
    idle_expires_at     timestamptz NOT NULL,
    revoked_at          timestamptz,
    revoked_reason      text,
    initial_ip          inet,
    last_ip             inet,
    initial_user_agent  text,
    last_user_agent     text,
    device_label        text
);

-- 每个 refresh token 一行；session_id 即 token family
CREATE TABLE auth_refresh_tokens (
    id             uuid PRIMARY KEY,
    session_id     uuid NOT NULL REFERENCES auth_sessions(id) ON DELETE CASCADE,
    token_hash     bytea NOT NULL UNIQUE,
    issued_at      timestamptz NOT NULL DEFAULT now(),
    expires_at     timestamptz NOT NULL,
    used_at        timestamptz,
    replaced_by    uuid REFERENCES auth_refresh_tokens(id),
    revoked_at     timestamptz,
    revoked_reason text
);

CREATE INDEX auth_refresh_tokens_session_idx
    ON auth_refresh_tokens (session_id);

CREATE INDEX auth_sessions_user_active_idx
    ON auth_sessions (user_id) WHERE revoked_at IS NULL;
```

设计说明：

- `auth_sessions.id` 同时作为 JWT 的 `sid` 和 refresh token family ID。
- `auth_refresh_tokens.session_id` 保留“哪个 token 属于哪个会话”的关系；reuse 时按 `session_id` 撤销整个 family。
- `used_at` + `replaced_by` 支持检测“旧 token 被再次使用”，也支持可选的幂等 grace 处理。
- `absolute_expires_at` 和 `idle_expires_at` 由服务端强制；cookie 的 `Max-Age` 只是辅助。
- `initial_ip`/`last_ip`/`user_agent` 用于设备展示、风险信号、审计；不要把它们当作唯一认证因子。
- 所有时间用 `timestamptz`，避免 `timestamp without time zone` 的时区歧义。参见 [PostgreSQL 18 Date/Time Types](https://www.postgresql.org/docs/18/datatype-datetime.html)。

### 3.4 登录与刷新流程

**登录**

1. 验证用户名/密码（见第 5、6 节）和 MFA。
2. 生成 `session_id`（UUID/ULID）和第一个 refresh token（32 字节随机）。
3. 插入 `auth_sessions`：`absolute_expires_at = now() + 12h`，`idle_expires_at = now() + 30min`。
4. 插入 `auth_refresh_tokens`：`token_hash = SHA-256(raw)`，`expires_at = absolute_expires_at`。
5. 签发 access JWT：`sid=session_id`、`ver=users.token_version`、TTL 10–15 分钟。
6. `Set-Cookie: __Host-refresh=...; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=...`。
7. 返回 access token（以及可选的 CSRF token）；**不要**在 JSON 中返回 refresh token。

**刷新**

```text
POST /auth/refresh
  cookie: __Host-refresh=<raw>
  header: X-Requested-With: XMLHttpRequest
  Origin: https://app.example.com

1. 校验 Origin + CSRF（见第 4 节）。
2. BEGIN;
3. SELECT s.*, t.*
   FROM auth_refresh_tokens t
   JOIN auth_sessions s ON s.id = t.session_id
   WHERE t.token_hash = SHA-256(raw)
   FOR UPDATE OF s, t;
4. 若找不到 -> 401（不泄漏原因）。
5. 若 s.revoked_at IS NOT NULL 或 s.absolute_expires_at <= now()
   或 s.idle_expires_at <= now() -> 撤销/拒绝，401。
6. 若 t.revoked_at IS NOT NULL -> 401。
7. 若 t.used_at IS NOT NULL -> reuse detection：
   - 撤销整个 session/family；
   - 记录 authn_token_reuse；
   - 通知用户；
   - 401。
8. 若 t.used_at IS NULL：
   - UPDATE auth_refresh_tokens SET used_at = now() WHERE id = t.id;
   - 生成新 refresh token，插入新行，replaced_by = 新 id；
   - UPDATE auth_sessions
       SET last_seen_at = now(),
           idle_expires_at = LEAST(now() + interval '30 minutes',
                                   absolute_expires_at)
     WHERE id = s.id;
   - 签发新 access token；
   - COMMIT；
   - Set-Cookie 新 refresh token；
   - 返回新 access token。
```

并发/重试注意：

- 用 `SELECT ... FOR UPDATE` 锁住 session 和 token 行，保证同一 refresh token 的并发请求串行化。
- 前端必须做 **single-flight**：同一时刻只允许一个 refresh 请求；其他请求等待它完成后重放。
- 网络故障可能导致“服务端已轮换、客户端没收到新 token”。可选的工程处理：
  - 设一个短 grace window（例如 30 秒）：旧 token 在 grace 内再次出现，且新 token 尚未被使用、会话未撤销时，返回同一个新 token（幂等），而不是直接判定 reuse；
  - 记录 `token_reuse_grace` 指标；grace 外或新 token 已被使用则按 reuse 撤销 family。
  - 这是安全性与可用性的权衡：grace 越长，攻击者利用旧 token 的窗口越大。RFC 9700 要求 rotation + 检测 replay，但没有规定 grace 时长；30 秒是工程建议，需按威胁模型评审。参见 [RFC 9700 §4.14.2](https://www.rfc-editor.org/rfc/rfc9700.html)。

### 3.5 Reuse detection 的语义

- RFC 9700 对 public client 要求 sender-constrained refresh token 或 **refresh token rotation**；rotation 时每次刷新都发新 refresh token、旧 token 失效，但服务端保留关系。若旧 token 被攻击者和合法客户端同时使用，一方会提交已失效 token，服务端据此发现泄漏并撤销 active refresh token。参见 [RFC 9700 §4.14.2](https://www.rfc-editor.org/rfc/rfc9700.html)。
- 本系统应撤销 **整个 token family / session**，而不只是当前 token；否则攻击者可以用 family 里的其他 token 继续。family 的边界就是 `auth_sessions.id`。
- Reuse 事件必须记录为高优先级审计事件（`authn_token_reuse`），并触发告警/用户通知。参见 [OWASP Logging Vocabulary](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Vocabulary_Cheat_Sheet.html)。
- 不能确定“谁是攻击者”；撤销 family 会强制合法用户重新登录，这是可接受的代价。参见 [RFC 9700 §4.14.2](https://www.rfc-editor.org/rfc/rfc9700.html)。

### 3.6 会话/设备记录与 revoke-all

**用户可见的会话/设备信息**

- `device_label`：从 User-Agent 解析出的浏览器/OS/设备类型；不要信任，仅展示。
- `initial_ip`/`last_ip`：展示最近登录位置；可用于风险提示，但 IP 会变，不能作为唯一认证因子。
- `created_at`/`last_seen_at`：显示“何时登录/最近活跃”。
- 提供“撤销此设备”“撤销其他所有设备”“退出所有设备”。参见 [ASVS V7 7.5.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。

**并发会话上限**

- 明确允许的并发会话数（例如 5 个），以及在达到上限时的行为（撤销最旧/拒绝新登录/提示用户）。ASVS 7.1.2 要求文档化。参见 [ASVS V7 7.1.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
- 超过上限记录 `excess_sessions_exceeded`。参见 [OWASP Logging Vocabulary](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Vocabulary_Cheat_Sheet.html)。

**Revoke-all**

```sql
BEGIN;
UPDATE auth_sessions
SET revoked_at = now(), revoked_reason = 'user_revoke_all'
WHERE user_id = $1 AND revoked_at IS NULL;

UPDATE auth_refresh_tokens
SET revoked_at = now(), revoked_reason = 'user_revoke_all'
WHERE session_id IN (SELECT id FROM auth_sessions WHERE user_id = $1)
  AND revoked_at IS NULL;

UPDATE users
SET token_version = token_version + 1
WHERE id = $1;
COMMIT;
```

- 撤销所有 refresh token 后，refresh 立即失败；access token 通过 `sid`/`ver` 校验失效（或最多等 TTL）。
- 改密/重置密码/禁用账户/删除账户时执行同样的撤销逻辑，并通知用户。参见 [ASVS V7 7.4.2/7.4.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
- 管理员需要能撤销单个用户或全部用户的会话。参见 [ASVS V7 7.4.5](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。

### 3.7 反模式

- ❌ 数据库存 refresh token 明文；一旦 DB 泄漏，攻击者可直接使用。
- ❌ Refresh token 用 JWT 且有效期 30 天，无服务端会话记录；无法撤销/reuse detection。
- ❌ 每次刷新都发新 token，但不保存旧 token 关系；无法检测 reuse。参见 [RFC 9700 §4.14.2](https://www.rfc-editor.org/rfc/rfc9700.html)。
- ❌ 检测到 reuse 只撤销当前 token，不撤销 family；攻击者继续用其他 token。
- ❌ 滑动会话没有 absolute cap；用户可永久保持登录。
- ❌ Refresh token 返回给 JavaScript 或存 localStorage。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- ❌ 只按 IP/UA 绑定 refresh token；移动网络/浏览器更新会导致误杀，且 IP 可伪造/共享。
- ❌ Revoke-all 只删 cookie，不更新数据库/`token_version`。
- ❌ 没有设备/会话列表，用户无法发现异常登录。参见 [ASVS V7 7.5.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。

---

## 4. 使用 credentials 时的 CSRF 与 CORS 防御

### 4.1 威胁模型：header token vs cookie token

- **Access token 放 `Authorization` header**：浏览器不会自动附加；跨站表单/图片/脚本无法设置该 header。因此业务 API 的 CSRF 风险很低。但跨域请求带 `Authorization` 会触发 CORS preflight，服务端必须正确配置 CORS。
- **Access/refresh token 放 Cookie**：浏览器会自动附加；攻击者只需让受害者浏览器向目标发请求即可，因此所有状态变更端点都需要 CSRF 防护。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。
- **CORS 不是 CSRF 防御**：CORS 限制的是“攻击者能否读取响应”，不是“请求是否被发送/执行”。一个 simple request（如 form POST）可以跨站发送并产生副作用，即使响应因 CORS 不可读。参见 [WHATWG Fetch](https://fetch.spec.whatwg.org/#cors-protocol) 与 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。

本系统推荐：**业务 API 用 header token；只有 refresh/logout 用 cookie**。这样 CSRF 防护面最小。

### 4.2 CORS 配置规则

| 规则 | 要求 | 依据 |
|---|---|---|
| Allow-Origin | 精确 allowlist（如 `https://app.example.com`）；不要反射任意 Origin | ASVS 3.4.2 |
| Allow-Credentials | 需要 cookie 时设为 `true`；此时 `Allow-Origin` 不能是 `*` | Fetch §3.3.5 |
| Allow-Methods | 只列实际使用的方法（GET/POST/PUT/PATCH/DELETE/OPTIONS） | Fetch |
| Allow-Headers | 明确列 `Authorization, Content-Type, X-CSRF-Token`；credentials 模式下 `*` 不等于通配 | Fetch |
| Expose-Headers | 只暴露前端需要的响应头 | Fetch |
| Max-Age | 预检缓存 5–10 分钟；密钥/配置轮换期间不要过长 | Fetch |
| Vary | 响应随 Origin 变化时必须 `Vary: Origin`，避免缓存污染 | Fetch / OWASP CSRF |
| 错误响应 | 4xx/5xx 也要带正确的 CORS 头，否则前端只看到网络错误 | 工程实践 |
| `null` origin | 不要 allowlist `null`（sandboxed iframe/file:// 可产生） | 工程实践 |

- Fetch 规范明确：credentials mode 为 `include` 时，`Access-Control-Allow-Origin` 必须是具体 origin；`*` 不允许，且必须同时有 `Access-Control-Allow-Credentials: true`。参见 [WHATWG Fetch §3.3.5](https://fetch.spec.whatwg.org/#cors-protocol-and-credentials)。
- ASVS 3.4.2 要求 `Access-Control-Allow-Origin` 是固定值或对 Origin 做 allowlist 校验；`*` 只允许用于不含敏感信息的响应。参见 [ASVS V3 3.4.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x12-V3-Web-Frontend-Security.md)。
- 预检请求本身不携带 credentials；但服务端仍要对实际请求做认证/授权。参见 [WHATWG Fetch](https://fetch.spec.whatwg.org/#cors-protocol-and-credentials)。

### 4.3 Refresh/logout 的 CSRF 防护

即使 access token 在 header 中，refresh/logout 仍使用 cookie，必须防护：

**推荐组合（按优先级）**

1. **`SameSite=Strict`（或 `Lax`）**：阻止大多数跨站携带；但 same-site 子域攻击仍可能携带，不能单独依赖。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
2. **精确 `Origin` allowlist**：对 refresh/logout 等状态变更请求，校验 `Origin` 必须是 `https://app.example.com`；缺失/不匹配则拒绝。不要只检查 `Referer`（可能被隐私设置去掉）。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。
3. **自定义请求头**：要求 `X-CSRF-Token` 或 `X-Requested-With: XMLHttpRequest`。跨站表单无法设置自定义头；跨域 fetch 会触发 preflight，preflight 会被 CORS allowlist 拦截。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。
4. **CSRF token（同步器模式）**：登录/refresh 响应返回一个与会话绑定的随机 token，前端存内存，放在 `X-CSRF-Token` header；服务端与 session 中的值做常量时间比较。注意冷启动/页面刷新后内存 token 丢失，第一次 refresh 无法携带它；refresh 端点仍需 Origin allowlist + 自定义 header + SameSite 作为主防线，或改用 BFF/同源部署。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。
5. **Fetch Metadata**：检查 `Sec-Fetch-Site`，把 `cross-site` 的状态变更请求拒绝；`same-site` 是否信任取决于是否信任同级子域。OWASP 建议把 `Sec-Fetch-Site` 作为主信号，但必须为不支持的老浏览器保留 Origin/CSRF token fallback。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。

**跨 origin 同 site 的一个重要细节**

- 如果 SPA 在 `app.example.com`、API 在 `api.example.com`，前端 JavaScript **读不到** API 域的 cookie（包括非 HttpOnly 的 CSRF cookie），因为 `document.cookie` 按 origin 隔离。因此：
  - 不要设计“JS 读取 API 的 CSRF cookie 再回传”的 double-submit；
  - 用登录/refresh 响应体返回 CSRF token（CORS 只允许可信 origin 读取），前端存内存；或
  - 直接用“自定义 header + 精确 Origin allowlist”，不依赖可读 cookie。
- 如果 SPA 与 API 同源，double-submit cookie 可行；但 signed double-submit 必须绑定 session，不能只用 naive 的随机 cookie 对比。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。

### 4.4 SameSite 与同级子域的盲点

- `SameSite` 只区分 site，不区分 origin。`evil.example.com` 与 `api.example.com` 是 same-site，因此 `SameSite=Strict/Lax` 的 refresh cookie 仍可能被 `evil.example.com` 发起的请求携带。
- 如果 `evil.example.com` 被攻陷，攻击者可以：
  - 向 `api.example.com` 发 simple request（form POST），cookie 自动携带；
  - 无法读取响应，但请求可能已执行（例如触发 refresh 轮换、logout）。
- 防御：
  - refresh/logout 校验精确 `Origin`（`https://app.example.com`），不信任 `same-site`；
  - 要求自定义 header/CSRF token；
  - 不在父域设置 `Domain` cookie；
  - 对同级子域做安全治理（避免任意子域被接管）。
- ASVS 3.5.4 建议不同应用使用不同 hostname，以利用 same-origin policy；但 hostname 分离不等于 site 分离，仍需 Origin/CSRF 校验。参见 [ASVS V3 3.5.4](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x12-V3-Web-Frontend-Security.md)。

### 4.5 反模式

- ❌ `Access-Control-Allow-Origin: *` + `Access-Control-Allow-Credentials: true`。参见 [WHATWG Fetch](https://fetch.spec.whatwg.org/#cors-protocol-and-credentials)。
- ❌ 反射请求的 `Origin` 而不做 allowlist。
- ❌ 允许 `Origin: null`。
- ❌ `Access-Control-Allow-Headers: *` 以为能匹配所有 header；credentials 模式下必须显式列。
- ❌ 只依赖 `SameSite=None` 防 CSRF；SameSite 不是 CSRF token 的替代品。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。
- ❌ 只检查 `Sec-Fetch-Site: same-site`，却信任不可信的同级子域。
- ❌ 用 GET 执行 logout/refresh 等状态变更；应使用 POST。参见 [ASVS V3 3.5.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x12-V3-Web-Frontend-Security.md)。
- ❌ CSRF token 放在 URL/query；会进日志/Referer。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。
- ❌ 登录后不轮换 CSRF token/session；存在 fixation 风险。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- ❌ CORS 只配置成功响应，错误响应缺少 CORS 头，前端无法区分 401/500。

---

## 5. Argon2id 参数与密码策略（NIST SP 800-63B）

### 5.1 Argon2id 参数

| 来源 | 内存 `m` | 迭代 `t` | 并行 `p` | Salt | Tag | 说明 |
|---|---|---|---|---|---|---|
| OWASP 最低配置 | 19 MiB（19456 KiB） | 2 | 1 | 每密码唯一 | 库默认 | 不得低于此 |
| RFC 9106 第二推荐 | 64 MiB（2^16 KiB） | 3 | 4 | 128-bit | 256-bit | 内存较少时的统一安全选项 |
| RFC 9106 第一推荐 | 2 GiB（2^21 KiB） | 1 | 4 | 128-bit | 256-bit | 高内存场景 |
| 本系统建议起点 | 64 MiB | 3 | 4（或 1，视 CPU） | 16 字节 | 32 字节 | benchmark 到约 100–300ms；不低于 OWASP 最低 |

- OWASP 明确要求 Argon2id 最低 `m=19 MiB, t=2, p=1`；并指出 Argon2id 是兼顾 side-channel 和 GPU 攻击的变体。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。
- RFC 9106 的第二推荐选项是 `t=3, p=4, m=2^16 (64 MiB), 128-bit salt, 256-bit tag`；第一推荐是 `t=1, p=4, m=2^21 (2 GiB)`。参见 [RFC 9106 §4](https://www.rfc-editor.org/rfc/rfc9106.html)。
- `p` 是 lane 数，不会把总内存乘以 p；不要用“p 越大越安全”来替代 `m`。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html) 与 [RFC 9106 §3.1](https://www.rfc-editor.org/rfc/rfc9106.html)。
- 选参后必须在生产硬件上 benchmark：目标是单次验证约 100–300ms，且不超过 1 秒；OWASP 指出工作因子要在安全与性能之间平衡，过高可被用来做 DoS。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html) 与 [ASVS V11 11.4.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x20-V11-Cryptography.md)。

### 5.2 Salt、pepper、PHC 与 rehash

- **Salt**：每个密码唯一、随机；RFC 9106 建议 128-bit（16 字节）salt。现代 Argon2 库通常自动生成并编码进 PHC 字符串。参见 [RFC 9106 §3.1](https://www.rfc-editor.org/rfc/rfc9106.html) 与 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。
- **PHC 字符串**：存储算法、版本、参数、salt、hash，例如：

  ```text
  $argon2id$v=19$m=65536,t=3,p=4$<base64-salt>$<base64-hash>
  ```

  这样可以在登录时判断参数是否落后，并用新参数 rehash。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html) 与 [ASVS V11 11.4.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x20-V11-Cryptography.md)。
- **Pepper（可选）**：在 Argon2 之前 HMAC/加密，或在 Argon2 之后再做一次 HMAC；pepper 不放数据库，放 KMS/Secrets Manager/HSM。NIST 建议额外的 keyed hash，密钥与密码哈希分开存储。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html) 与 [NIST SP 800-63B-4 §3.1.1.2](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- **Pepper 轮换**：需要知道用户密码才能重新计算，通常要强制重置密码或双写过渡；不要把 pepper 轮换当作无成本操作。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。
- **算法升级**：登录成功时检查 PHC 参数，若低于当前策略则用新参数 rehash；保留旧 hash 直到用户下次登录。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。
- **DoS 防护**：Argon2id 每次验证分配 `m` 内存；64 MiB × 并发数会迅速吃满内存。用信号量/队列限制并发 hash 数，登录限速，内存不足时返回 429/503 而不是 OOM。OWASP 明确提醒工作因子过高可被用于 DoS。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。

### 5.3 密码策略

| 规则 | 推荐 | 依据 |
|---|---|---|
| 单因子密码最短长度 | 15 字符 | NIST 800-63B-4 §3.1.1.2；ASVS 6.2.1（最短 8，强烈建议 15） |
| 有 MFA 时最短长度 | 8 字符（管理员仍建议 15） | NIST 800-63B-4 §3.1.1.2 |
| 最大长度 | 至少允许 64；建议 128 字符 | NIST 800-63B-4；ASVS 6.2.9 |
| 字符组成 | 不要求大小写/数字/符号组合 | NIST 800-63B-4；ASVS 6.2.5 |
| 定期轮换 | 不要求；仅在怀疑泄漏/ compromise 时强制 | NIST 800-63B-4；ASVS 6.2.10 |
| Blocklist | 检查常见/已泄漏密码；至少 top 3000，最好接入 Pwned Passwords | ASVS 6.2.4/6.2.12；NIST 800-63B-4 |
| Unicode | 接受；按 code point 计长度；哈希前做 NFC | NIST 800-63B-4 |
| 截断 | 不截断；完整验证用户输入 | ASVS 6.2.8；NIST 800-63B-4 |
| 粘贴/密码管理器 | 允许 | ASVS 6.2.7；NIST 800-63B-4 |
| 密码提示/安全问题 | 禁止 | ASVS 6.4.2；NIST 800-63B-4 |
| 改密验证 | 需要当前密码 + 新密码 | ASVS 6.2.3 |
| 改密通知 | 通知用户 | ASVS 6.3.7 |
| MFA | 管理员强制 WebAuthn/TOTP；email 不作为认证因子 | ASVS 6.3.3/6.3.6；NIST 800-63B-4 |

- NIST 800-63B-4 明确：单因子密码最短 15 字符；仅用于 MFA 的密码可短至 8 字符；应允许至少 64 字符；不得强加组合规则；不得要求定期更换；必须检查 blocklist；不得截断；Unicode 按 code point 计数并在哈希前 NFC。参见 [NIST SP 800-63B-4 §3.1.1.2](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- ASVS 5.0 6.2.1 要求最短 8、强烈建议 15；6.2.9 要求允许至少 64；6.2.4 要求至少 top 3000 密码 blocklist；6.2.12 要求检查已泄漏密码；6.2.5 禁止组合规则；6.2.10 禁止定期轮换。参见 [ASVS V6](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- 密码最大长度建议 128 字符：既能满足 NIST/ASVS 的 64 下限，也能限制极端输入的哈希成本；超过上限时返回明确错误，**不要截断**。参见 [NIST SP 800-63B-4 §3.1.1.2](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- 不要用 bcrypt 的 72-byte 限制来约束 Argon2；Argon2 没有该限制。若历史系统用 bcrypt，需按 72 字节限制并避免预哈希陷阱。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。
- 管理员 MFA：优先 WebAuthn/passkey（抗钓鱼），TOTP 作为备选；不要用 email OTP 作为管理员第二因子。参见 [ASVS V6 6.3.3/6.3.6](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md) 与 [NIST SP 800-63B-4](https://pages.nist.gov/800-63-4/sp800-63b.html)。

### 5.4 注册、改密与重置

- **注册/改密**：检查 blocklist（top 3000 + breached），拒绝则给出可操作原因；不要泄露“该密码在某次泄漏中出现”的具体来源（可给通用提示）。参见 [NIST SP 800-63B-4 §3.1.1.2](https://pages.nist.gov/800-63-4/sp800-63b.html) 与 [ASVS V6 6.2.4/6.2.12](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- **改密**：要求当前密码；成功后撤销其他会话、递增 `token_version`、通知用户。参见 [ASVS V6 6.2.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md) 与 [ASVS V7 7.4.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
- **忘记密码**：生成 ≥128-bit 随机 token，数据库只存 hash；单次使用；15–30 分钟过期；不泄露账户是否存在；发送链接而非临时密码；成功后撤销所有会话并通知。参见 [ASVS V6 6.4.1/6.4.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md) 与 [OWASP Forgot Password](https://cheatsheetseries.owasp.org/cheatsheets/Forgot_Password_Cheat_Sheet.html)。
- **管理员重置**：管理员可以发起重置，但不能知道/选择用户新密码；用一次性链接或临时密码 + 强制改密。参见 [ASVS V6 6.4.6](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。

### 5.5 反模式

- ❌ MD5/SHA-1/SHA-256 单次哈希（无 salt/无 KDF）存密码。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。
- ❌ Argon2id 参数低于 OWASP 最低（19 MiB/t=2/p=1）。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。
- ❌ 所有用户共用 salt，或 salt 可预测。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。
- ❌ Pepper 存在数据库里，和 hash 一起被拖走。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)。
- ❌ 强制“大写+数字+符号”、定期 90 天改密、安全问题。参见 [NIST SP 800-63B-4 §3.1.1.2](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- ❌ 密码截断到 72/64 字节；用户不知道，且降低熵。参见 [ASVS V6 6.2.8](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- ❌ 用 email OTP 作为管理员 MFA。参见 [ASVS V6 6.3.6](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- ❌ 登录/重置响应泄露账户是否存在。参见 [ASVS V6 6.3.8](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- ❌ 在日志中记录密码、密码 hash、reset token。参见 [ASVS V16 16.2.5](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。

---

## 6. 无 Redis 的登录限速与账户锁定（PostgreSQL）

### 6.1 目标与分层

| 层 | key | 目的 | 建议阈值（示例） |
|---|---|---|---|
| Account | HMAC(username/email) | 防单账户暴力破解/密码喷洒 | 5 次失败后 30 秒，10 次后 5 分钟，15 次后 1 小时 + 通知 |
| IP | HMAC(IP，IPv6 取 /64) | 防单源批量尝试 | 20 次失败/10 分钟 → 15 分钟 |
| Global | 固定 key | 检测大规模攻击/异常 | 监控 + 告警；必要时全站 CAPTCHA/延迟 |
| Device/ASN | 可选 | 识别分布式/代理网络 | 风险评分，不单独阻断 |

- NIST 要求限制单个账户/认证器的连续失败次数，**不超过 100 次**后禁用认证器；允许更严格。参见 [NIST SP 800-63B-4 §3.2.2](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- OWASP 建议账户锁定计数器绑定账户本身，而不是只绑 IP，以防攻击者用大量 IP 绕过；同时提醒锁定可被用于 DoS。参见 [OWASP Authentication](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)。
- 仅靠 per-IP 限速不足以应对分布式 credential stuffing；应结合账户层、设备指纹、MFA/CAPTCHA。参见 [OWASP Credential Stuffing Prevention](https://cheatsheetseries.owasp.org/cheatsheets/Credential_Stuffing_Prevention_Cheat_Sheet.html)。

### 6.2 表结构

```sql
CREATE TABLE auth_login_throttle (
    scope           text        NOT NULL CHECK (scope IN ('account','ip','global')),
    key_hash        bytea       NOT NULL,
    window_start    timestamptz NOT NULL,
    failures        integer     NOT NULL,
    locked_until    timestamptz,
    last_failure_at timestamptz NOT NULL,
    PRIMARY KEY (scope, key_hash)
);

CREATE INDEX auth_login_throttle_cleanup_idx
    ON auth_login_throttle (last_failure_at);
```

- `key_hash` 用 HMAC-SHA-256(pepper, canonical_key)，避免在限速表里存明文用户名/IP；未知用户和已存在用户使用同样的 key 计算方式，避免通过限速行为枚举账户。
- `scope` + `key_hash` 作为主键；每个 (scope,key) 一行，避免表按尝试次数无限增长。
- `timestamptz` 存 UTC；所有窗口判断用数据库 `now()`，不要用应用服务器本地时间。参见 [PostgreSQL 18 Date/Time Types](https://www.postgresql.org/docs/18/datatype-datetime.html)。

### 6.3 原子 upsert

```sql
INSERT INTO auth_login_throttle AS t
    (scope, key_hash, window_start, failures, last_failure_at)
VALUES
    ($1, $2, now(), 1, now())
ON CONFLICT (scope, key_hash) DO UPDATE
SET failures = CASE
        WHEN t.window_start <= now() - $3::interval THEN 1
        ELSE t.failures + 1
    END,
    window_start = CASE
        WHEN t.window_start <= now() - $3::interval THEN now()
        ELSE t.window_start
    END,
    last_failure_at = now()
RETURNING failures, window_start, locked_until;
```

- PostgreSQL 明确保证 `INSERT ... ON CONFLICT DO UPDATE` 是原子的“插入或更新”，即使高并发也只会得到两种结果之一。参见 [PostgreSQL 18 INSERT](https://www.postgresql.org/docs/18/sql-insert.html)。
- `t.failures` 在 `DO UPDATE` 的 `SET` 表达式中引用已存在行；`RETURNING` 返回更新后的行。不要用 `SELECT count(*)` 再 `INSERT` 的非原子模式，否则并发请求可能同时低于阈值。
- 若需要同时更新 account + IP 两行并保持严格一致，可在更新前获取事务级 advisory lock：

  ```sql
  SELECT pg_advisory_xact_lock(hashtextextended($key, 0));
  ```

  Advisory lock 是应用定义的锁，事务结束自动释放；不要跨事务持有。参见 [PostgreSQL 18 Advisory Lock Functions](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADVISORY-LOCKS)。
- 不要在进行 Argon2 验证时持有 advisory lock（约 100ms+），否则会串行化该账户所有登录并放大 DoS；锁只包住计数/锁定更新。

### 6.4 判定流程

```text
on_login(identifier, password, ip, ua):
  account_key = HMAC(pepper, "account:" + canonical(identifier))
  ip_key      = HMAC(pepper, "ip:" + normalize_ip(ip))

  1. 读取 account/ip 的 locked_until；若任一未过期：
     - 仍然执行一次 dummy Argon2 verify（时间均衡）；
     - 返回 429 + Retry-After，通用错误信息；
     - 记录 authn_login_lock。
  2. 查询用户；无论用户是否存在，都执行 Argon2 verify：
     - 存在：verify(password, stored_hash)；
     - 不存在：verify(password, dummy_hash)（固定 dummy hash）。
  3. 若验证失败：
     - upsert account failures；
     - upsert ip failures；
     - 计算 lock_until（指数退避）；
     - 记录 authn_login_fail / authn_login_fail_max / authn_login_lock；
     - 返回 401 或 429（统一策略，见 6.5）。
  4. 若验证成功：
     - 清除 account 计数（DELETE 或 failures=0）；
     - IP 计数可保留（防止“成功一次就重置”绕过）；
     - 检查 MFA；
     - 创建 session/refresh token；
     - 记录 authn_login_success。
```

- 对未知用户执行 dummy Argon2 verify，避免通过响应时间枚举账户。参见 [OWASP Authentication](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html) 与 [ASVS V6 6.3.8](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- NIST 建议成功认证后重置该认证器的失败计数；本系统只重置 account 计数，保留 IP 计数用于检测持续滥用。参见 [NIST SP 800-63B-4 §3.2.2](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- 统一错误信息/状态码，避免通过 401/404/429 或响应时间判断账户是否存在。参见 [ASVS V6 6.3.8](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- 返回 429 时设置 `Retry-After`；响应加 `Cache-Control: no-store`。参见 [OWASP Authentication](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)。

### 6.5 锁定策略

| 失败次数（account） | 动作 | 说明 |
|---|---|---|
| 3 | 要求 CAPTCHA / 风险检查 | 仍允许尝试；记录风险 |
| 5 | 锁定 30 秒 | 指数退避起点 |
| 10 | 锁定 5 分钟 | 继续指数增长 |
| 15 | 锁定 1 小时 + 通知用户 | 高置信度滥用 |
| 反复触发 | 管理员介入/临时禁用 + 告警 | 不做永久自动锁定，避免 DoS |

- OWASP 建议指数退避（例如 1 秒起，每次翻倍），并考虑锁定阈值/时长/延迟；同时提醒锁定可被用于 DoS。参见 [OWASP Authentication](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)。
- NIST 允许在达到上限后禁用认证器，并要求重新绑定；这比“永久锁定账户”更精确。参见 [NIST SP 800-63B-4 §3.2.2](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- 提供恢复路径：通过已验证的 MFA/邮箱链接/管理员解锁；不要仅靠“忘记密码”自动解锁，否则攻击者可利用重置流程绕过锁定。参见 [OWASP Authentication](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)。
- 对管理员账户单独更严格策略 + 强制 MFA + 实时告警。参见 [OWASP Credential Stuffing Prevention](https://cheatsheetseries.owasp.org/cheatsheets/Credential_Stuffing_Prevention_Cheat_Sheet.html)。

### 6.6 清理与可观测性

- 定期删除过期记录：

  ```sql
  DELETE FROM auth_login_throttle
  WHERE last_failure_at < now() - interval '30 days'
    AND (locked_until IS NULL OR locked_until < now());
  ```

  用 `last_failure_at` 索引；批量删除可分批（`LIMIT`）避免长事务。表大小和清理延迟要监控。
- 记录限速指标：按 scope 的失败率、锁定数、429 比例、CAPTCHA 通过率、独立 IP/账户数；用于检测攻击和误伤。参见 [OWASP Credential Stuffing Prevention](https://cheatsheetseries.owasp.org/cheatsheets/Credential_Stuffing_Prevention_Cheat_Sheet.html)。
- 多实例部署时，PostgreSQL 是共享权威状态；本地内存缓存只能作为加速，不能作为唯一计数。若用本地缓存，必须接受短时间不一致，并定期回写/读取 DB。
- 代理场景只信任来自已知反代的 `X-Forwarded-For`；否则攻击者可伪造 IP。参见 [ASVS V4 4.1.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x13-V4-API-and-Web-Service.md)。

### 6.7 反模式

- ❌ 只用 per-IP 限速；分布式代理可绕过。参见 [OWASP Credential Stuffing Prevention](https://cheatsheetseries.owasp.org/cheatsheets/Credential_Stuffing_Prevention_Cheat_Sheet.html)。
- ❌ 用 `SELECT count(*)` + `INSERT` 的竞态实现；并发下会放过超额尝试。
- ❌ 永久自动锁定账户；攻击者可故意锁死管理员（DoS）。参见 [OWASP Authentication](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)。
- ❌ 锁定后立即返回“账户不存在/已锁定”的差异信息；可枚举账户。参见 [ASVS V6 6.3.8](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- ❌ 未知用户直接返回，不执行 dummy hash；响应时间泄露账户存在性。参见 [OWASP Authentication](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)。
- ❌ 成功登录后重置所有层（包括 IP）计数；攻击者可交替成功/失败绕过。
- ❌ 限速表不清理，无限增长。
- ❌ 信任客户端传入的 `X-Forwarded-For`。参见 [ASVS V4 4.1.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x13-V4-API-and-Web-Service.md)。
- ❌ 只记录失败，不记录锁定/限速事件，无法告警。参见 [OWASP Logging](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)。

---

## 7. 审计事件分类与保留策略

### 7.1 事件命名

优先使用 OWASP Logging Vocabulary 的既有名称，便于 SIEM/告警复用；自定义事件用一致前缀并在文档中登记。

| 类别 | 事件示例 | 说明 |
|---|---|---|
| 登录 | `authn_login_success`, `authn_login_fail`, `authn_login_successafterfail`, `authn_login_fail_max`, `authn_login_lock` | 成功/失败/达到阈值/锁定 |
| 密码 | `authn_password_change`, `authn_password_change_fail`; 自定义 `authn_password_reset_requested`, `authn_password_reset_completed` | 改密/重置 |
| Token | `authn_token_created`, `authn_token_revoked`, `authn_token_reuse` | 创建/撤销/重放检测 |
| 会话 | `session_created`, `session_renewed`, `session_expired`, `session_logout`, `session_use_after_expire`, `excess_sessions_exceeded` | 会话生命周期 |
| 授权 | `authz_fail`, `authz_change`, `authz_admin` | 失败/权限变更/管理员操作 |
| 账户管理 | 自定义 `admin_user_created`, `admin_user_updated`, `admin_user_disabled`, `admin_user_deleted`, `admin_role_assigned`, `admin_role_revoked` | 管理后台关键操作 |
| 密钥/配置 | 自定义 `crypto_key_rotated`, `crypto_key_activated`, `crypto_key_retired`, `config_security_changed` | 轮换/配置变更 |
| 审计本身 | 自定义 `audit_log_accessed`, `audit_log_exported` | 审计访问也要审计 |

- OWASP Logging Vocabulary 定义了 `authn_login_success`、`authn_login_fail`、`authn_login_fail_max`、`authn_login_lock`、`authn_password_change`、`authn_token_created`、`authn_token_revoked`、`authn_token_reuse`、`authz_fail`、`authz_admin`、`session_created`、`session_renewed`、`session_expired`、`session_logout`、`session_use_after_expire`、`excess_sessions_exceeded` 等。参见 [OWASP Logging Vocabulary](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Vocabulary_Cheat_Sheet.html)。
- ASVS 16.3.1 要求记录所有认证操作（成功和失败）；16.3.2 要求记录授权失败；16.3.3 要求记录文档化安全事件和绕过安全控制的尝试。参见 [ASVS V16](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。

### 7.2 每条审计记录的字段

| 字段 | 说明 |
|---|---|
| `event` | 事件名（如 `authn_login_fail`） |
| `timestamp` | UTC RFC 3339，毫秒精度；NTP 同步 |
| `severity` | `info`/`warning`/`error`/`critical`，与告警映射 |
| `service`, `env`, `host` | 服务、环境、实例 |
| `request_id`, `trace_id` | 关联同一请求/链路 |
| `actor_id`, `actor_type` | 用户/管理员/系统；不要放 email 等不必要 PII |
| `target_user_id` | 被操作的用户（管理操作） |
| `session_id` | 会话 ID（可用内部 ID；不要放 refresh token） |
| `source_ip`, `user_agent` | 请求来源；注意隐私与代理可信性 |
| `method`, `path`, `status_code` | HTTP 上下文；path 不含 query secret |
| `outcome`, `reason_code` | 成功/失败及内部原因码（对外不暴露） |
| `auth_method`, `mfa_method` | 密码/TOTP/WebAuthn 等 |
| `jti_hash`, `kid` | token 审计/密钥轮换关联；jti 可 hash |
| `duration_ms` | 性能/异常检测 |

- ASVS 16.2.1 要求日志包含 when/where/who/what 等调查所需元数据；16.2.2 要求时间源同步、时间戳用 UTC 或显式时区（推荐 UTC）；16.2.5 要求按数据保护级别处理敏感数据，凭据/支付数据不得记录，session token 只能 hash/mask。参见 [ASVS V16 16.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。
- OWASP Logging Cheat Sheet 同样要求记录 when/where/who/what，并建议使用结构化、可关联的格式。参见 [OWASP Logging](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)。
- **绝不记录**：密码、密码 hash、refresh token、access token、Cookie、Authorization header、MFA secret、reset token、完整身份证/手机号等。参见 [ASVS V16 16.2.5](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。
- 对 `jti`/session ID 可做 HMAC/hash 后记录，既保留关联能力又降低泄漏风险。参见 [OWASP Logging](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)。

### 7.3 保留策略

保留期必须由组织策略、合同和适用法规决定；以下是**示例基线**，不是通用法规结论：

| 类别 | 热存储（可查询） | 温/归档 | 说明 |
|---|---|---|---|
| 认证/授权审计（登录、改密、token、管理员操作） | 90 天 | 1 年 | 安全调查最低基线示例 |
| 高风险事件（token reuse、密钥轮换、权限变更） | 180 天 | 3 年 | 视合规要求延长 |
| 调试/应用日志 | 7–30 天 | 不需要 | 不应包含敏感数据 |
| 法律保全（litigation hold） | 按法务要求 | 按法务要求 | 停止自动删除 |

- ASVS 16.1.1 要求有日志清单，记录每层日志、事件、格式、存储位置、用途、访问控制以及**保留多久**。参见 [ASVS V16 16.1.1](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。
- NIST SP 800-53 AU-11 要求按组织定义的保留期保留审计记录，以支持事后调查并满足法规/组织保留要求。参见 [NIST SP 800-53 Rev.5 AU-11](https://csrc.nist.gov/pubs/sp/800/53/r5/upd1/final)。
- NIST SP 800-92 要求日志保留/处置与组织策略、法律/法规要求一致，并在保留期结束后正确销毁。参见 [NIST SP 800-92](https://csrc.nist.gov/pubs/sp/800/92/final)。
- OWASP Logging 明确：日志、临时调试日志、备份/副本/导出不得在保留期前销毁，也不得超过保留期继续保存。参见 [OWASP Logging](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)。
- 日志含个人信息时，遵循数据最小化、访问控制、删除/更正流程；具体法定义务（如中国的《个人信息保护法》、GDPR）需法务确认。本文不对具体法定年限作结论。

### 7.4 完整性、访问控制与告警

- 审计日志应发送到与业务系统逻辑隔离的集中系统；应用被攻陷时日志不应一起被篡改。参见 [ASVS V16 16.4.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。
- 日志写入账号应只具备追加/写入权限，不能更新/删除历史；对归档可加 hash chain/WORM/对象锁。参见 [ASVS V16 16.4.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md) 与 [OWASP Logging](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)。
- 关键告警：`authn_token_reuse`、`authn_login_fail_max`/`authn_login_lock` 激增、管理员角色变更、密钥轮换失败、审计日志写入失败、异常地理/设备登录。参见 [OWASP Logging](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)。
- 审计日志访问本身也要记录（谁看了/导出了什么），防止内部滥用。参见 [ASVS V16 16.3.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。

### 7.5 反模式

- ❌ 记录密码/token/cookie/Authorization header。参见 [ASVS V16 16.2.5](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。
- ❌ 非结构化文本、本地时间、无 request ID，无法关联。
- ❌ 审计事件采样/丢弃；审计日志必须完整。
- ❌ 只存在应用数据库，管理员可 UPDATE/DELETE。参见 [ASVS V16 16.4.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。
- ❌ 没有保留/删除策略；永久保存或过早删除。参见 [NIST SP 800-53 AU-11](https://csrc.nist.gov/pubs/sp/800/53/r5/upd1/final)。
- ❌ 不记录授权失败/管理员操作；只看登录成功。参见 [ASVS V16 16.3.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。
- ❌ 有日志但没有告警/值班流程。参见 [OWASP Logging](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)。

---

## 8. JWT 密钥轮换

### 8.1 密钥层次与算法

- **非对称签名**：ES256（P-256）或 EdDSA（Ed25519）；私钥只用于签名，公钥用于验证。参见 [RFC 8725 §3.2](https://www.rfc-editor.org/rfc/rfc8725.html) 与 [ASVS V11 11.2.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x20-V11-Cryptography.md)。
- **密钥分离**：
  - 每个环境（dev/staging/prod）独立密钥；
  - access token 签名密钥与 ID token、密码重置 token、邮件验证 token 的密钥分开；
  - 不与 refresh token 共用（refresh 是不透明串，不需要签名密钥）。
  - 目的：缩小泄漏影响，防止 cross-JWT confusion。参见 [RFC 8725 §3.12](https://www.rfc-editor.org/rfc/rfc8725.html) 与 [ASVS V9 9.1.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md)。
- **`kid`**：每个 key 有唯一、不可复用的 ID；JWT header 带 `kid`。`kid` 只是选择提示，不能作为信任来源。参见 [RFC 7515 §4.1.4](https://www.rfc-editor.org/rfc/rfc7515.html) 与 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html)。
- **私钥存储**：KMS/HSM/Secrets Manager；不要提交到 repo、不要放 `.env` 明文、不要进日志、不要放数据库明文。参见 [OWASP Key Management](https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html) 与 [ASVS V11 11.1.1](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x20-V11-Cryptography.md)。
- **公钥分发**：单服务可用配置/数据库中的公钥集合；多服务/未来扩展用 JWKS（RFC 7517），只暴露公钥。参见 [RFC 7517](https://www.rfc-editor.org/rfc/rfc7517.html)。

### 8.2 常规轮换流程（publish-before-use）

1. 在 KMS 生成新密钥对，分配新 `kid`（如 `2027-05-01-a`）。
2. 将新公钥加入 JWKS/公钥集合，**保留旧公钥**；等待所有验证方缓存刷新。缓存 `max-age` 建议 5–10 分钟，因此至少等待一个缓存周期 + 余量（例如 15 分钟）。
3. 切换签名器：新签发的 access token 使用新私钥和新 `kid`。
4. 旧公钥继续用于验证，至少覆盖 `access token 最大 TTL + clock skew`。本系统 access token 15 分钟，建议保留旧公钥 ≥ 1 小时；如果有其他 JWT（ID token/refresh JWT）使用同一 key，按最长 TTL 保留。
5. 监控验证失败、未知 `kid`、401 率；确认旧 token 已过期后，从 JWKS 移除旧公钥。
6. 记录 `crypto_key_rotated` 审计事件（新旧 `kid`、操作者、时间、审批单号）。参见 [OWASP Key Management](https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html) 与 [ASVS V11 11.1.1](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x20-V11-Cryptography.md)。

**关键点**：先发布公钥，再切换签名；不要先删旧 key 再切新 key，否则旧 token 全部失效（除非你就是想强制全量下线）。

### 8.3 应急轮换（私钥泄漏/怀疑泄漏）

1. **立即停止**用旧私钥签名；从 JWKS 移除旧公钥（或标记 revoked），使旧 token 立即失效（fail-closed）。
2. 生成新密钥对并发布公钥；切换签名器。
3. 强制所有用户重新登录：撤销所有 `auth_sessions`，递增全局/每用户 `token_version`；必要时轮换 refresh token pepper。
4. 保留旧私钥的审计/取证副本（加密、受限访问），但不再用于验证；调查访问日志、异常签发、管理员操作。
5. 通知安全团队/用户，按事件响应流程处理；记录 `crypto_key_rotated`（reason=`compromise`）。

- NIST 800-57 指出密钥泄漏后 cryptoperiod 立即失效，必须停止使用并评估影响。参见 [NIST SP 800-57 Part 1 Rev.5 §5.3/§5.5](https://csrc.nist.gov/pubs/sp/800/57/pt1/r5/final)。
- ASVS 7.4.2 要求账户禁用/删除时终止所有会话；密钥泄漏的全量失效同理。参见 [ASVS V7 7.4.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
- 不要为了“平滑”而继续信任已泄漏的旧公钥；攻击者可以用旧私钥伪造任意 token。

### 8.4 JWKS 缓存与验证方行为

- JWKS 端点只返回公钥；设置 `Cache-Control: public, max-age=300`（5 分钟）或类似短缓存，避免轮换延迟过长。
- 验证方按 `kid` 在已信任的 JWKS 中查找；遇到未知 `kid` 时最多刷新一次 JWKS（限速），仍找不到则拒绝。
- 不要把 token 里的 `jku`/`jwk`/`x5u` 当作 JWKS 地址或 key 来源；只使用配置中固定的 issuer JWKS。参见 [ASVS V9 9.1.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md)。
- 多 issuer 场景按 `iss` 绑定 key set，不要在所有 issuer 的 key 并集里按 `kid` 搜索。参见 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html) 与 [RFC 8725 §3.8](https://www.rfc-editor.org/rfc/rfc8725.html)。
- 如果只有本服务验证 token，可把公钥集合放在配置/数据库中；仍建议保留 `kid` 和 overlap 流程，便于未来拆分服务。

### 8.5 Cryptoperiod 建议

- NIST SP 800-57 Table 1 建议 private signature key 的 originator-usage period 为 **1–3 年**；public signature-verification key 可用数年。参见 [NIST SP 800-57 Part 1 Rev.5 Table 1](https://csrc.nist.gov/pubs/sp/800/57/pt1/r5/final)。
- OWASP Key Management 引用同一范围：digital signature private key 通常 1–3 年；并建议自动化轮换、记录手动轮换。参见 [OWASP Key Management](https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html)。
- 对本系统：建议**每年至少一次**常规轮换（在 NIST 1–3 年范围内取较短值），并保留应急轮换能力；如果运维自动化成熟，可缩短到 90–180 天。不要频繁到造成大面积故障。
- 轮换周期必须文档化，并纳入密钥清单/加密资产 inventory。参见 [ASVS V11 11.1.1/11.1.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x20-V11-Cryptography.md)。

### 8.6 反模式

- ❌ 一把密钥用多年、无 `kid`、无轮换计划。参见 [NIST SP 800-57](https://csrc.nist.gov/pubs/sp/800/57/pt1/r5/final)。
- ❌ 轮换时先删除旧公钥，导致所有未过期 token 立即失效（非计划全量掉线）。
- ❌ 复用 `kid`；验证方无法区分新旧 key。
- ❌ 私钥提交到 Git、写在 `.env`、打进容器镜像、输出到日志。参见 [OWASP Key Management](https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html)。
- ❌ 所有环境共用一把 key；dev 泄漏导致 prod 可伪造。
- ❌ 用 HS256 且把 secret 放在 SPA/前端环境变量；前端无法保守秘密。参见 [OWASP JWT Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html)。
- ❌ 接受 token 里的 `jku`/`jwk` 作为 key 来源。参见 [ASVS V9 9.1.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md)。
- ❌ 轮换没有监控/告警；未知 `kid` 暴增才发现。
- ❌ 私钥泄漏后继续“平滑过渡”，保留旧公钥信任。参见 [NIST SP 800-57 §5.3](https://csrc.nist.gov/pubs/sp/800/57/pt1/r5/final)。

---

## 9. 落地检查清单（按优先级）

**P0（上线前必须）**

- [ ] 全站 HTTPS + HSTS；HTTP 自动跳转仅用于用户入口。参见 [ASVS V3 3.4.1](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x12-V3-Web-Frontend-Security.md)。
- [ ] Refresh cookie：`Secure; HttpOnly; SameSite=Strict/Lax`；优先 `__Host-`；无 `Domain`。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- [ ] Access token 只在内存；不落 localStorage/sessionStorage。参见 [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)。
- [ ] Access JWT：`exp` 10–15 分钟；验证 `iss`/`aud`/`exp`/`nbf`/`typ`；`alg` allowlist；`kid` 来自受信任 key set。参见 [ASVS V9](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md)。
- [ ] Refresh token：256-bit 随机；DB 只存 hash；每次刷新轮换；reuse 撤销 family；服务端会话记录。参见 [RFC 9700 §4.14.2](https://www.rfc-editor.org/rfc/rfc9700.html)。
- [ ] Logout/改密/禁用账户：撤销 refresh session + 递增 `token_version`；access token 通过 `sid`/`ver` 失效。参见 [ASVS V7 7.4.1/7.4.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
- [ ] CORS：精确 origin allowlist；credentials 时禁止 `*`；`Vary: Origin`。参见 [WHATWG Fetch](https://fetch.spec.whatwg.org/#cors-protocol-and-credentials)。
- [ ] Refresh/logout：POST + Origin allowlist + 自定义 header/CSRF token；SameSite 仅作纵深。参见 [OWASP CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)。
- [ ] 密码：Argon2id ≥ OWASP 最低；最短 15（单因子）/8（MFA）；最长 ≥64；blocklist；无组合规则/定期轮换。参见 [OWASP Password Storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html) 与 [NIST SP 800-63B-4](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- [ ] 登录限速：account + IP + global；原子 upsert；指数退避；无永久自动锁定；未知用户 dummy hash。参见 [NIST SP 800-63B-4 §3.2.2](https://pages.nist.gov/800-63-4/sp800-63b.html)。
- [ ] 审计：认证成功/失败、token reuse、锁定、改密、管理员操作、密钥轮换；UTC JSON；不记录 secret。参见 [ASVS V16](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)。
- [ ] JWT 密钥：非对称 + `kid`；私钥 KMS/HSM；轮换有 overlap；应急流程。参见 [OWASP Key Management](https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html)。

**P1（上线后尽快）**

- [ ] 管理员强制 MFA（WebAuthn/TOTP）。参见 [ASVS V6 6.3.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)。
- [ ] 会话/设备列表 + 撤销单设备/全部设备。参见 [ASVS V7 7.5.2](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
- [ ] 敏感操作 step-up reauth。参见 [ASVS V7 7.5.1/7.5.3](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
- [ ] 异常登录通知、token reuse 告警、管理员操作告警。
- [ ] 密钥轮换自动化 + 演练（含应急轮换）。
- [ ] 审计保留/归档/销毁自动化；法务确认保留期。
- [ ] 生产硬件 Argon2 benchmark + 并发内存预算/限流。
- [ ] 浏览器矩阵测试：Chrome 普通/Incognito、Safari、移动端。

---

## 10. 不确定性与待确认项

1. **Chrome 第三方 Cookie 政策仍在变化**：截至 2026-11-05，普通模式保留用户选择、Incognito 默认阻止；未来可能再变。上线前用目标浏览器实测，不要把 third-party cookie 作为唯一通道。参见 [Chrome Help](https://support.google.com/chrome/answer/95647) 与 [Privacy Sandbox 2025-10-17](https://privacysandbox.google.com/blog/update-on-plans-for-privacy-sandbox-technologies)。
2. **RFC 6265bis 尚未分配 RFC 号**：`draft-ietf-httpbis-rfc6265bis-22` 已获 IESG 批准并在 RFC Editor 队列；最终 RFC 文本可能有编辑性变化。实现以浏览器实际行为为准，并跟踪 RFC 发布。
3. **ASVS 5.0.0 是稳定基线**：GitHub `latest`（2026-10-26）包含未发布条目，本文未引用；若后续升级 ASVS，需要重新映射条款。
4. **NIST 800-63B-4 的 TTL 是 AAL2 建议**：本系统按管理后台取更严格值（12h/30min）；若产品定位不是 AAL2，需重新做风险分级并记录 deviation。参见 [ASVS V7 7.1.1](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)。
5. **限速阈值/锁定策略是工程建议**：5/10/15 次、30s/5min/1h 只是示例，需结合用户规模、误伤成本、客服能力、攻击面调整；不要机械照搬。
6. **Argon2 参数需生产 benchmark**：64 MiB/t=3/p=4 在低配容器上可能造成内存压力；需测 P95 登录延迟、并发内存、OOM 风险，再定 `m`/`p`/并发上限。
7. **Refresh rotation grace window 是权衡**：30 秒 grace 降低网络重试误杀，但也扩大旧 token 可用窗口；高安全场景可取消 grace，改用客户端单飞 + 失败重登录。RFC 9700 未规定具体 grace 时长。
8. **审计保留年限未做法律调研**：本文只给示例基线；中国《个人信息保护法》、GDPR、行业监管/合同可能有不同要求，需法务确认。
9. **未覆盖**：OAuth/OIDC 第三方登录、Passkey/WebAuthn 具体实现、mTLS/DPoP sender-constrained token、WAF/风控系统、PostgreSQL 行级安全（RLS）、多租户隔离。这些若进入范围，需单独调研。
10. **未写生产代码**：本文所有 SQL/配置为设计示意，落地时需 SQLx 迁移、事务边界、错误处理、压测和安全测试。

---

## 11. 主要来源

### OWASP

- [OWASP ASVS 5.0.0 release](https://github.com/OWASP/ASVS/releases/tag/v5.0.0_release)
- [ASVS V3 Web Frontend Security](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x12-V3-Web-Frontend-Security.md)
- [ASVS V4 API and Web Service](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x13-V4-API-and-Web-Service.md)
- [ASVS V6 Authentication](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)
- [ASVS V7 Session Management](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md)
- [ASVS V9 Self-contained Tokens](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x18-V9-Self-contained-Tokens.md)
- [ASVS V11 Cryptography](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x20-V11-Cryptography.md)
- [ASVS V16 Security Logging and Error Handling](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md)
- [OWASP Session Management Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)
- [OWASP CSRF Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)
- [OWASP Password Storage Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)
- [OWASP Authentication Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)
- [OWASP JSON Web Token Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_Cheat_Sheet.html)
- [OWASP Logging Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)
- [OWASP Logging Vocabulary Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Vocabulary_Cheat_Sheet.html)
- [OWASP Credential Stuffing Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Credential_Stuffing_Prevention_Cheat_Sheet.html)
- [OWASP Key Management Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html)
- [OWASP Forgot Password Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Forgot_Password_Cheat_Sheet.html)

### NIST

- [NIST SP 800-63B-4, Digital Identity Guidelines: Authentication and Lifecycle Management](https://pages.nist.gov/800-63-4/sp800-63b.html)
- [NIST SP 800-57 Part 1 Rev. 5, Recommendation for Key Management](https://csrc.nist.gov/pubs/sp/800/57/pt1/r5/final)
- [NIST SP 800-92, Guide to Computer Security Log Management](https://csrc.nist.gov/pubs/sp/800/92/final)
- [NIST SP 800-53 Rev. 5, AU-11 Audit Record Retention](https://csrc.nist.gov/pubs/sp/800/53/r5/upd1/final)

### IETF RFC / Draft

- [RFC 9106, Argon2 Memory-Hard Function for Password Hashing and Proof-of-Work Applications](https://www.rfc-editor.org/rfc/rfc9106.html)
- [RFC 7519, JSON Web Token (JWT)](https://www.rfc-editor.org/rfc/rfc7519.html)
- [RFC 6750, The OAuth 2.0 Authorization Framework: Bearer Token Usage](https://www.rfc-editor.org/rfc/rfc6750.html)
- [RFC 9700, Best Current Practice for OAuth 2.0 Security](https://www.rfc-editor.org/rfc/rfc9700.html)
- [RFC 8725, JSON Web Token Best Current Practices](https://www.rfc-editor.org/rfc/rfc8725.html)
- [RFC 9068, JSON Web Token (JWT) Profile for OAuth 2.0 Access Tokens](https://www.rfc-editor.org/rfc/rfc9068.html)
- [RFC 7517, JSON Web Key (JWK)](https://www.rfc-editor.org/rfc/rfc7517.html)
- [RFC 7515, JSON Web Signature (JWS)](https://www.rfc-editor.org/rfc/rfc7515.html)
- [draft-ietf-httpbis-rfc6265bis-22, Cookies: HTTP State Management Mechanism](https://www.ietf.org/archive/id/draft-ietf-httpbis-rfc6265bis-22.html)

### Web platform / browsers

- [WHATWG HTML: same-site](https://html.spec.whatwg.org/multipage/browsers.html#same-site)
- [WHATWG Fetch: CORS protocol](https://fetch.spec.whatwg.org/#cors-protocol)
- [WebKit Tracking Prevention Policy](https://webkit.org/tracking-prevention/)
- [Chrome Help: Delete, allow, and manage cookies](https://support.google.com/chrome/answer/95647)
- [Privacy Sandbox: Update on plans for Privacy Sandbox technologies (2025-10-17)](https://privacysandbox.google.com/blog/update-on-plans-for-privacy-sandbox-technologies)
- [Chrome for Developers: Cookies Having Independent Partitioned State (CHIPS)](https://privacysandbox.google.com/cookies/chips)

### PostgreSQL 18

- [PostgreSQL 18: INSERT / ON CONFLICT](https://www.postgresql.org/docs/18/sql-insert.html)
- [PostgreSQL 18: Advisory Lock Functions](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADVISORY-LOCKS)
- [PostgreSQL 18: Date/Time Types](https://www.postgresql.org/docs/18/datatype-datetime.html)
