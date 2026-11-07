# ADR-0006: 前端认证集成与路由守卫

- Status: accepted
- Date: 2026-11-07

## Context

前端是 `satnaing/shadcn-admin` 的裁剪 fork（审计快照 `e16c87f`）。上游认证是纯客户端 mock：access token 存在 JS 可读 Cookie、没有 route guard、没有 API 层、用户信息硬编码；刷新页面后只有 token 恢复、user 永远回到 null。裁剪后必须替换 mock 层，而不是改配置。

本 ADR 在既有决议之上决定 SPA 的认证集成：

- ADR-0001：access JWT 15 分钟，只存 JS 内存，通过 `Authorization: Bearer` 发送；refresh 是 `__Host-refresh` HttpOnly Cookie，轮换 + 30 秒 grace + 复用检测；`sid`/`ver` 每请求校验；客户端必须单飞刷新。
- ADR-0002：`GET /auth/me` 返回有效权限与 `is_super_admin`；前端复用同一权限目录门控菜单与按钮；服务端始终权威。
- ADR-0004：API 前缀 `/api/v1`；错误为 RFC 7807 ProblemDetails（稳定 `code`）；`POST /auth/login` 返回 `UserSelf`，`POST /auth/refresh` 只返回 token；`GET /auth/me` 是用户与权限的权威来源；orval 8 + axios custom mutator 生成并提交客户端。
- ADR-0005：step-up 以最近 300 秒认证为准，失败返回 403 `reauth_required`；登录、刷新、登出等端点的 CSRF 要求。
- 「角色管理 API 与 UI」决议：step-up 被动触发；共享重新认证对话框；成功一次后重试原操作一次；403 页面保留；无权限不渲染菜单。

## Decision

### 1. 认证状态归属与冷启动

- Zustand `auth-store` 只保存 `accessToken` 与 `authStatus`，**不使用 `persist` 中间件**；不写入 localStorage、sessionStorage 或 JS 可读 Cookie。
- 当前用户与权限由 TanStack Query 的 `['auth','me']` 持有，数据来自 `UserSelf`；登录响应里的 `user` 直接写入该 query，refresh 响应不包含用户。
- 根路由 `beforeLoad` 统一执行 `bootstrap()`（单飞）：
  1. 若已有内存 token 与 `['auth','me']`，直接复用；
  2. 否则 `POST /auth/refresh`（`withCredentials` + CSRF 头）；
  3. 成功后保存 access token，再 `GET /auth/me` 写入 query cache；
  4. refresh 返回 401 时静默置为未登录；
  5. refresh 或 `/me` 遇到网络/5xx 时不误判为登出，保持 `unknown` 并允许重试；公开路由仍可渲染，受保护路由显示可重试的引导错误。
- `/sign-in` 的 `beforeLoad` 在已认证时跳转到净化后的 `redirect` 或 `/`；未认证时正常渲染登录页。
- 登录成功、登出、刷新失败时清空 query cache，避免跨用户残留数据。

### 2. API client 与 base URL

- 使用 orval 8 生成的 React Query hooks，生成物提交仓库；所有请求经过同一个 axios custom mutator，注入 Bearer、处理错误与 401 刷新。CI 用 `git diff --exit-code` 防生成物漂移（ADR-0004）。
- API 地址由 `VITE_API_BASE_URL` 注入：
  - 生产占位 `https://api.example.com/api/v1`（实际域名由部署票据确定）；
  - 开发默认 `/api/v1`，由 Vite dev proxy 转发到后端，避免本地跨源 Cookie/CORS 噪音。
- axios 默认 `withCredentials: false`；仅以下端点使用 `withCredentials: true`（必要时配合 ADR-0005 §7.4 的 CORS credentials 允许清单）：
  - `POST /auth/login`、`POST /auth/refresh`、`POST /auth/logout`；
  - `POST /auth/logout-all`（清除 refresh Cookie）；
  - `POST /auth/change-password`（轮换 refresh Cookie）。
- `POST /auth/login`、`POST /auth/refresh`、`POST /auth/logout`、`POST /auth/logout-all` 发送 `X-Requested-With: XMLHttpRequest` 与 `Content-Type: application/json`；`Origin` 由浏览器自动携带。`POST /auth/reauthenticate` 不读取 refresh Cookie，不发送 CSRF 头。
- 所有错误转换为统一的 `ApiError`：保留 `status`、`code`、`request_id`、`errors[].pointer/code/params`、`Retry-After`；非 ProblemDetails 响应（网关 HTML、代理错误）回退为按状态码分类的通用错误。上游 `handleServerError` 对 `{title}` 的假设被替换，不再作为错误文案来源。
- 若 Bun/Node 版本实测无法运行 orval 8.46，按 ADR-0004 的回退方案改用 `openapi-typescript` + `openapi-fetch`；mutator 的认证、错误与刷新契约保持不变，不退化为手写业务客户端。

### 3. 401 刷新、并发去重与重试

- 响应拦截器维护模块级 `refreshPromise`：并发 401 共享同一次 `POST /auth/refresh`，只触发一次令牌轮换。
- 业务请求收到 401（`access_token_invalid` / `session_invalid`）时：
  1. 若该请求已重试过，直接失败；
  2. 否则等待 `refreshPromise`；
  3. 成功后更新内存 token，并**只重试原请求一次**。
- 不触发 401 刷新的端点：`POST /auth/login`、`POST /auth/refresh`、`POST /auth/reauthenticate`、`POST /auth/logout`。`GET/PATCH /auth/me`、`POST /auth/change-password`、`POST /auth/logout-all` 仍可在 401 后刷新并重试一次。
- 刷新失败（401 `refresh_token_invalid` / `refresh_reuse_detected`、网络错误或 5xx）时：清空 auth store 与 query cache；若当前不在 `/sign-in`，跳转 `/sign-in?redirect=<当前站内路径>`；只提示一次「会话已过期」。
- 移除上游 `QueryCache` 的 401 处理，避免与拦截器重复 toast/跳转；401、403、429、500、503 的错误展示收敛到 API client 与全局错误映射。
- 不做按 `expires_in` 的定时主动刷新；刷新由冷启动与 401 被动触发。
- 查询默认值：401/403/429 永不重试；网络错误与 5xx 的只读查询最多重试 1 次；mutation 不自动重试。401 后刷新再重试原请求是安全的，因为请求在鉴权阶段已被拒绝。

### 4. 路由守卫、403 与受限会话

- `_authenticated` 根路由的 `beforeLoad` 按顺序执行：
  1. `bootstrap()`；
  2. 未认证 → `redirect('/sign-in', { redirect: 当前站内路径 })`；
  3. `must_change_password === true` 且目标不是 `/change-password` → `redirect('/change-password', { redirect: 当前站内路径 })`；
  4. 路由声明的权限检查。
- `redirect` 参数只接受站内相对路径：必须以单个 `/` 开头，拒绝 `//`、反斜杠、协议/主机、控制字符，以及 `/sign-in` 自循环；无法解析时回退 `/`。
- 权限不足时跳转到真实 `/403?from=<原路径>` 页面；不显示空列表、不静默重定向。`/401`、`/403`、`/404`、`/500`、`/503` 为公开错误页，不再套认证守卫。
- `/change-password` 是需要认证的路由：未认证 → 登录页并保留目标；已认证且 `must_change_password` 为 false 时也可用于自助改密。改密成功后刷新 `['auth','me']`，回到 `redirect` 目标或 `/`。
- 权限映射：
  - `/`（dashboard）、`/settings/*`、`/change-password`：仅要求登录；
  - `/users`：`users.read`；
  - `/roles`、`/roles/$roleId`：`roles.read`；
  - `/roles/new`：`roles.create`；
  - `/audit-logs`：`audit.read`。
- 业务 API 返回 403 `password_change_required` 时跳转 `/change-password`；返回 403 `account_disabled` 时清空会话并回登录页提示账号已停用。403 `reauth_required` 由重新认证 helper 处理，不当作权限不足或 401。

### 5. 登录、登出与错误体验

- 登录表单只保留「标识（username 或 email）+ 密码」；移除 sign-up、forgot-password、OTP、社交登录按钮与无效 Terms/Privacy 链接。
- 错误按稳定 `code` 映射，不使用后端 `title`/`detail` 作为主要文案：
  - 401 `invalid_credentials`：统一的「标识或密码错误」，不区分用户是否存在；
  - 403 `account_disabled`：账号已停用；
  - 403 `temporary_password_expired`：临时密码已过期，需要管理员重置；
  - 429 `login_locked` / `rate_limited`：读取 `Retry-After` 倒计时并禁用提交；
  - 422：字段级错误；
  - 网络/5xx：通用连接或服务错误，显示 `request_id` 便于排查。
- 登录成功：清空旧 query cache → 保存 access token → 写入 `['auth','me']` → 按净化后的 `redirect` 跳转；若 `must_change_password` 为 true，跳 `/change-password?redirect=<原目标>`。
- 登出：确认框 → `POST /auth/logout`（带 CSRF 头）→ **无论请求成功与否**清空 auth store 与 query cache → 跳 `/sign-in`。请求失败时额外提示「本机已退出，服务器会话可能仍有效」。`POST /auth/logout-all` 类似，失败时明确提示未完成全部撤销。
- 多标签页只广播登出（`BroadcastChannel('rs-admin-auth')`）：其他标签页清空状态；位于受保护路由时跳登录，公开页只清状态。access token 不跨标签共享；刷新令牌轮换由服务端 grace 窗口兜底。
- 登录、登出都不写入分析日志或持久化存储；密码字段不进入 query cache。

### 6. 权限门控

- `useAuth()` 从 `['auth','me']` 读取用户；`hasPermission(key)` 在 `is_super_admin === true` 时恒真，否则检查 `permissions.includes(key)`。
- 侧栏 nav item 增加可选 `permission` 字段，`useVisibleSidebarData()` 过滤；缺少权限时**隐藏**该项；命令菜单消费同一份过滤数据。目标级不可操作（`assignable`/`manageable` 为 false）才使用禁用 + tooltip，与「无权限」区分。
- 操作按钮使用 `<Can permission="...">` 或 `usePermission`；缺少动作权限时隐藏，目标级不可操作时禁用并说明原因。服务端仍是最终裁决。
- 路由权限映射集中在 `route-permissions` 配置，不在各页面重复判断；直接访问无权限路由显示 403。
- `['auth','me']` 在登录、`PATCH /auth/me` 成功、改密成功后更新/失效；生产环境开启 `refetchOnWindowFocus`，`staleTime` 30 秒，不轮询。收到 `forbidden` / `out_of_scope` 时可失效一次 `['auth','me']` 以纠正过期权限，但不自动跳转。
- 导航门控只依赖 `/me.permissions`；不在启动时为了菜单额外拉 `GET /permissions`。权限目录的完整清单仍用于角色矩阵等管理页面。

### 7. 共享的重新认证 helper

- 全局 `ReauthProvider` 与 Promise 式对话框；`requestReauth()` 单飞：并发 403 共享同一个弹窗与同一次 `POST /auth/reauthenticate`。
- `runWithReauth(fn)` 捕获 `ApiError.code === 'reauth_required'`：
  1. 若已有重新认证在进行，等待它；
  2. 否则打开对话框；成功后更新内存 access token；
  3. **重试 `fn()` 一次**；第二次仍 `reauth_required` 则直接抛出，不循环。
- 取消对话框时 reject `ReauthCancelledError`；调用方保留表单状态、不执行原操作。密码错误 422 `invalid_current_password` 在对话框内联提示；429 显示 `Retry-After` 倒计时；403 `password_change_required` 关闭对话框并跳 `/change-password`。
- 重新认证成功后的 5 秒内再次收到 `reauth_required`（通常是认证前已发出的并发请求）直接重试、不再弹窗，用于吸收竞态。**不解码** access token 的 `auth_time` 做主动预判；服务端 `auth_sessions.authenticated_at` 始终权威。
- 角色保存的 PATCH + PUT 共享一次 step-up：顺序执行时第一个请求触发弹窗，第二个在 300 秒窗口内直接通过；并发时单飞弹窗 + 5 秒竞态窗口保证不二次弹窗。
- 只有可能要求 step-up 的 mutation 使用 `runWithReauth`；GET 请求与登录/刷新/登出/重新认证本身不包装。

### 8. Settings 子页的数据归属

- **Profile**：只编辑 `display_name`（`PATCH /auth/me`）；username、email、phone、status、roles 只读，并标注 username/email/phone 由管理员维护。移除上游 bio、URLs 与硬编码 email 选项。
- **Account**：改密表单（当前密码、新密码、确认）调 `POST /auth/change-password`；成功后更新 access token、失效并重新拉取 `['auth','me']`，提示其他设备已退出；若原本是受限会话，回到登录前目标。提供「退出所有设备」按钮调 `POST /auth/logout-all`。语言选择保留在 Account，但只写本地 `locale` Cookie（`zh-CN`/`en-US`），不接后端；文案与格式化实现见 ADR-0007。
- **Appearance**：保留主题/字体本地偏好；v1 仅 LTR，不再提供用户可见的方向切换，`DirectionProvider` 与逻辑 CSS 保留，`dir` 固定为 `ltr`（见 ADR-0007 §7）；不接后端。
- **Notifications / Display**：保留页面，但只做本地偏好（localStorage），明确标注 v1 不接后端、不发送通知；Display 的侧栏偏好可应用到侧栏，不新增 API。移除上游 `showSubmittedData` 作为生产提交路径。
- 强制改密使用独立 `/change-password` 路由，不复用 Settings 的修改流程；改密成功后的跳转由第 4 节的路由守卫处理。

### 9. 测试与验收

- 单元测试：`hasPermission`、redirect 净化、auth store、401 单飞刷新、`runWithReauth` 单飞/取消/只重试一次。
- 集成测试（Vitest browser mode + MSW）：登录成功与错误矩阵、冷启动 refresh 成功/401/网络失败、401 刷新后重试与刷新失败登出、未登录/无权限/受限会话三类守卫、侧栏与按钮门控、Profile 保存、改密、重新认证对话框。
- 不在本票据引入 E2E；真实后端联调与部署相关的端到端验证由部署/测试票据承担。
- 验收点：未登录不能进入受保护页；已登录访问 `/sign-in` 被带回目标；并发 401 只刷新一次；`reauth_required` 只弹一次且成功后重试一次；取消重新认证不执行操作且表单保留；无权限路由显示 403；`must_change_password` 只能先改密；登出后本机状态立即清空。

## Amendments

- 2026-11-07：[前端裁剪与中英 i18n](https://github.com/nasa-du/rs-admin-ds/issues/13) 修订 §8：
  - Appearance 不再提供手动 LTR/RTL 切换；v1 仅 LTR，`DirectionProvider` 与逻辑 CSS 保留，locale→dir 映射固定为 `ltr`（ADR-0007 §7）。
  - Account 的语言选择只支持 `zh-CN`/`en-US`，写入本地 `locale` Cookie；文案、日期/数字格式化与错误码映射的实现见 ADR-0007。
- 2026-11-07：[OpenAPI 3.1 与生成 TS 客户端流水线](https://github.com/nasa-du/rs-admin-ds/issues/25) 修订 §2：
  - `VITE_API_BASE_URL` 的值改为 API 源（`scheme://host[:port]`，不含 `/api/v1`），因为生成的客户端路径已带完整 `/api/v1` 前缀（ADR-0004 2026-11-07 修订）。生产占位 `https://api.example.com`；开发默认留空，Vite dev proxy 转发 `/api/v1` 与 `/health` 到 `http://localhost:8080`。

## Consequences

- **正面**：token 只存内存，刷新与撤销语义清晰；冷启动统一走 refresh → `/me`，不需要在客户端持久化用户或权限；401 单飞与重新认证单飞避免弹窗/刷新风暴；路由与 UI 门控共享 `/me` 的权威权限；Settings 的数据归属明确，Notifications/Display 不再伪装成生产功能。
- **负面**：完整刷新页面必然多一次 refresh 与 `/me` 请求；权限变更需要重新拉 `/me` 才能反映到导航；跨标签只同步登出，其他标签在下次请求前可能仍显示旧界面；`change-password` 与 `logout-all` 需要后端 CORS credentials 支持（本决议修订 ADR-0005 §7.4）。
- **复查条件**：若冷启动请求量或延迟不可接受，评估在路由层做更细的懒引导；若权限变更需要近实时反映，评估定期 `refetchOnWindowFocus` 之外的主动失效；若多标签出现会话不一致事故，扩展 BroadcastChannel 到登录/刷新事件；若 step-up 竞态窗口产生误判，重新评估 5 秒窗口或改为服务端返回的 `auth_time` 只用于竞态去重。

## Considered Options

- **Zustand 同时保存 token 与 UserSelf**：实现简单，但权限与用户会出现两份副本，刷新/角色变更时容易不一致；不选。
- **手写 API client 与 hooks**：控制力强，但放弃 ADR-0004 已选定的生成契约，错误/刷新逻辑容易被各页面重复实现；不选。
- **按 `expires_in` 定时主动刷新**：减少一次 401 往返，但在休眠、时钟漂移、多标签下不可靠，且会掩盖会话撤销；不选。
- **在 TanStack Query 层统一处理 401**：与生成客户端的 mutator 职责重叠，容易重复 toast/跳转；不选，收敛到 API client。
- **权限不足时内联渲染 403 组件**：URL 不变，用户难以判断当前页面是否可访问；不选，使用真实 `/403` 路由。
- **每个页面各自实现重新认证对话框**：重复且行为不一致；不选，使用共享 provider。
- **解码 access token 的 `auth_time` 主动预判**：违反 #10 的被动触发结论，且会把令牌年龄误当成服务端会话状态；不选。
- **菜单项禁用而不是隐藏**：会让无权限用户看到大量不可用入口，且与 #10 的结论冲突；不选。
- **Notifications/Display 接后端**：v1 没有通知服务与用户偏好 API，新增契约收益低；不选，保持本地偏好并明确标注。
- **多标签共享 access token**：增加存储与同步复杂度，且 XSS 面更大；不选，服务端刷新 grace 已覆盖并发轮换。

## References

- [前端认证集成与路由守卫](https://github.com/nasa-du/rs-admin-ds/issues/12)
- [ADR-0001: 认证令牌与会话模型](./0001-auth-token-session-model.md)
- [ADR-0002: RBAC 权限模型与授权规则](./0002-rbac-permission-model.md)
- [ADR-0003: 用户生命周期与凭据管理](./0003-user-lifecycle-and-credential-management.md)
- [ADR-0004: API 契约与数据模型](./0004-api-and-data-contract.md)
- [ADR-0005: 安全基线](./0005-security-baseline.md)
- [角色管理 API 与 UI](https://github.com/nasa-du/rs-admin-ds/issues/10)
- `docs/research/shadcn-admin-audit.md`
