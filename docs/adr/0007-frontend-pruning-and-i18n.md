# ADR-0007: 前端裁剪、i18n 与 bun

- Status: accepted
- Date: 2026-11-07

## Context

前端是 `satnaing/shadcn-admin` 的裁剪 fork（审计快照 `e16c87f`）。上游是纯 UI 模板：认证为客户端 mock、无路由守卫、无 API 层、无 i18n、文案硬编码英文；RTL 基础设施存在且大量使用逻辑 CSS；包管理器为 pnpm（见 `docs/research/shadcn-admin-audit.md`）。

ADR-0006 已决定真实认证集成、路由守卫、权限门控与 Settings 数据归属；ADR-0002/0004/0005 决定权限目录、错误码与安全基线。「角色管理 API 与 UI」「审计日志查询 UI」「前端认证集成与路由守卫」又分别要求把 Roles/Audit Logs 导航、`/403`、`/change-password`、登录页裁剪与错误码文案纳入本票据。

本 ADR 决定 fork 后的最终页面面、路由树/侧栏/命令菜单、i18n 选型与文案组织、错误码本地化边界、RTL 去留、bun 迁移与许可归属。它只交付规格与决策，不写生产代码（地图「生产可用登录系统实施规格」的 Out of scope）。

## Decision

### 1. 最终页面清单与裁剪

保留：

| 路由 | 页面/用途 | 权限/说明 |
|---|---|---|
| `/` | Dashboard 占位壳 | 仅要求登录；不接真实业务数据 |
| `/users` | Users | `users.read`；真实 CRUD 由「Users 页面真实 CRUD 集成」实现 |
| `/roles` | 角色列表 | `roles.read` |
| `/roles/new` | 创建角色 | `roles.create` |
| `/roles/$roleId` | 角色详情/编辑 | `roles.read` |
| `/audit-logs` | 审计日志查询 | `audit.read` |
| `/settings` | Settings 布局 | 仅要求登录 |
| `/settings/` | Profile | 仅要求登录 |
| `/settings/account` | Account | 仅要求登录 |
| `/settings/appearance` | Appearance | 仅要求登录 |
| `/settings/notifications` | Notifications | 仅要求登录；只做本地偏好 |
| `/settings/display` | Display | 仅要求登录；只做本地偏好 |
| `/sign-in` | 登录 | 公开 |
| `/change-password` | 强制改密/自助改密 | 需要认证；公开错误页之外唯一使用认证布局的受保护页 |
| `/401`、`/403`、`/404`、`/500`、`/503` | 错误页 | 公开，不套认证守卫 |

删除：`tasks`、`apps`、`chats`、`help-center`、`sign-in-2`、`sign-up`、`forgot-password`、`otp`、`clerk/**`、`/_authenticated/errors/$error` 错误演示容器。研究审计曾建议删除上游 `/403` 及 `src/features/errors/forbidden.tsx`，但「角色管理 API 与 UI」与「前端认证集成与路由守卫」均要求保留真实 403 页面；本 ADR 以保留为准，`/403` 使用保留的 `forbidden-error` 组件并本地化其文案。

裁剪细则：

- **Dashboard**：保留路由与受保护占位壳，移除假指标、假销售记录、随机图表与 `Recent Sales` 头像引用；占位内容只显示本地化欢迎语与按权限可见的快捷入口。删除 `recharts` 依赖。
- **TeamSwitcher**：删除；v1 没有多团队/多租户概念，`sidebarData.teams` 一并移除。
- **Users 假数据**：删除 `@faker-js/faker`、`src/features/users/data/users.ts` 与相关 fixture；保留路由、共享 data-table 组件与空态/骨架，真实列表由「Users 页面真实 CRUD 集成」接入。相关测试 fixture 改为固定对象/MSW。
- **上游残留 UI**：删除 TopNav 的 Customers/Products 禁用链接、`nav-user`/`profile-dropdown` 的 Billing/Upgrade to Pro/New Team、社交登录按钮、无效 Terms/Privacy 链接、`src/assets/brand-icons/**`、Clerk logo/品牌素材、`src/components/learn-more.tsx`、`src/components/coming-soon.tsx`、`src/components/ui/input-otp.tsx`。依赖删除 `@clerk/react`、`input-otp`、`@faker-js/faker`、`recharts`；保留 `date-fns`（date-picker/calendar）、`react-day-picker`、`cmdk`（command menu）等仍有生产用途的依赖。删除后清理未使用的 icon imports，避免 lint/knip 报错。
- **品牌与许可**：界面名称、`index.html` 标题与文档标题使用 `rs-admin`；移除上游 logo/名称与 Clerk 赞助徽章；保留上游 `LICENSE` 原文（MIT，`Copyright (c) 2024 Sat Naing`），新增 `THIRD-PARTY-NOTICES.md`，README 注明 “Based on satnaing/shadcn-admin (MIT), snapshot e16c87f”。`public/images/*` 随 MIT 分发保留归属；替换品牌资源时避免暗示官方关联。
- **调试辅助**：`showSubmittedData` 不再作为生产提交路径；如保留仅限开发调试。
- **Dashboard/Users 占位**：占位壳不能伪造业务数据；空态需说明真实数据将在后续实现接入，不显示“暂无数据”之外的假统计。

### 2. 路由树、侧栏与命令菜单

- 路由组织沿用 TanStack Router file-based routing：
  - `src/routes/__root.tsx`：全局错误边界、`notFoundComponent`、`errorComponent`、`DocumentTitle`。
  - `src/routes/(auth)/route.tsx`：`AuthLayout`（居中卡片）；`sign-in.tsx`、`change-password.tsx` 为 children。
  - `src/routes/(errors)/401.tsx`、`403.tsx`、`404.tsx`、`500.tsx`、`503.tsx`：公开错误页。
  - `src/routes/_authenticated/route.tsx`：按 ADR-0006 执行 `bootstrap()`、登录检查、`must_change_password` 检查、路由权限检查，并渲染应用外壳。
  - `_authenticated/` 下：`index.tsx`（Dashboard）、`users/index.tsx`、`roles/index.tsx`、`roles/new.tsx`、`roles/$roleId.tsx`、`audit-logs/index.tsx`、`settings/route.tsx` 与五个子页。
- `/change-password` 放在 `(auth)` 分组，使用认证布局，不套应用侧栏外壳；其 `beforeLoad` 调用共享的认证 helper（`bootstrap()` + 未登录重定向），已认证但 `must_change_password=false` 时也允许自助改密。`_authenticated` 守卫把受限会话重定向到 `/change-password?redirect=<原路径>`；`/change-password` 自身不会触发循环重定向。
- `/sign-in` 已认证时跳转净化后的 `redirect` 或 `/`；`/sign-in` 的 `beforeLoad` 只做已认证判断，不要求 `users.read` 等业务权限。
- 公开错误页不套认证守卫；`/403` 由权限守卫跳转 `?from=<原路径>`，页面提供返回与联系管理员说明，不提供“切换账号”按钮。
- 侧栏分组与顺序：
  - Dashboard（单项，无需业务权限）。
  - Management（`nav.groups.management`）：Users → Roles → Audit Logs；分别按 `users.read`、`roles.read`、`audit.read` 显示，缺少权限时隐藏而非禁用。
  - Settings（`nav.groups.settings`）：Profile、Account、Appearance、Notifications、Display；仅要求登录。
- 侧栏数据从模块级常量改为 `useSidebarData()`（携带 `titleKey` 与可选 `permission`）和 `useVisibleSidebarData()`（按 `/me.permissions` 与 `is_super_admin` 过滤）；`CommandMenu` 消费同一份过滤数据，不再单独维护链接。删除的页面不得出现在侧栏、命令菜单、面包屑或 profile 菜单中。
- 命令菜单保留主题命令（Light/Dark/System），不新增语言命令；语言切换只在 Settings/Account。
- 路由权限映射集中在 `route-permissions` 配置，与 ADR-0006 §4/§6 一致；页面不得重复判断。
- 每个路由声明 `staticData.titleKey`，供文档标题与面包屑使用；错误页使用自己的标题 key。

### 3. i18n 选型、资源组织与初始化

- 选型：`i18next` + `react-i18next`。理由：生态成熟、支持命名空间/插值/复数、TypeScript key 类型增强、与 React 19 兼容；不选 `react-intl` 的 ICU 方案（团队需另学 MessageFormat，且与后端稳定 code 映射没有额外收益）。
- 资源：两种语言全部静态打包，不使用 `i18next-http-backend`、不按语言懒加载。管理员后台体积可控，静态资源避免首次切换的异步竞态、FOUC 与额外 CSP `connect-src`。
- 目录：
  - `src/i18n/index.ts`：初始化、`SUPPORTED_LOCALES`、`resolveInitialLocale()`、`setLocale()`、`i18n` 导出。
  - `src/i18n/locales/zh-CN/<namespace>.json`、`src/i18n/locales/en-US/<namespace>.json`。
  - `src/i18n/types.d.ts`：从 zh-CN 资源生成 `CustomTypeOptions`，提供严格 key 类型。
  - namespace：`common`、`nav`、`auth`、`users`、`roles`、`audit`、`settings`、`errors`、`validation`。
- 初始化：
  - `lng` 由 `resolveInitialLocale()` 决定；`fallbackLng: 'en-US'`；`supportedLngs: ['zh-CN', 'en-US']`；`load: 'currentOnly'`；`defaultNS: 'common'`；`ns` 显式列出全部 namespace。
  - `initImmediate: false`，保证静态资源同步初始化；在 `main.tsx` 渲染前 import `src/i18n`。
  - `interpolation: { escapeValue: false }`（React 负责转义）；`returnNull: false`。
  - 开发/测试环境 `missingKeyHandler` 报错（见 §9）；生产环境回退到 `en-US`，再缺失则显示 key，不静默吞掉。
- 复数使用 i18next 的 `Intl.PluralRules` 后缀（如 `_one`/`_other`），不手写单复数；中文只有 `_other`，英文需要 `_one`/`_other` 的键由 en-US 资源提供。
- 禁止运行时拼接 key（包括 `errors.${code}` 这类模板字符串）；错误码、审计动作等动态 code 必须经过静态映射表，未知 code 走显式 fallback。
- 资源格式用嵌套 JSON；键路径用 `.` 分隔，namespace 用 `:` 分隔（默认配置）。

### 4. 语言解析、持久化与切换

- 初始语言解析顺序：
  1. 本地 `locale` Cookie，值必须是受支持 locale；非法/缺失则忽略。
  2. `navigator.languages` 中第一个 `zh*` → `zh-CN`，第一个 `en*` → `en-US`。
  3. 默认 `zh-CN`。
- 偏好存储：Cookie `locale`，`Path=/; Max-Age=31536000; SameSite=Lax`，不设 `Domain`、不设 `HttpOnly`（需要前端读写），不设 `Secure`（非敏感偏好，且本地 HTTP 开发需要）；不写入后端用户资料。
- 切换语言：`i18n.changeLanguage` 立即生效，不刷新页面；同步更新 Cookie、`<html lang>`、`document.documentElement.dir`（固定 `ltr`）与文档标题。语言切换不改变路由、不重置表单，也不重新请求 API。
- Account 语言选项只保留「简体中文 / English」两项，用各自语言显示（endonym）；不显示上游的 fr/de/es/pt/ru/ja/ko 等未支持语言。选项变更只写本地偏好。
- `<html lang>` 初始值 `zh-CN`，运行时由 `LocaleProvider`/语言变更监听同步；`dir` 固定 `ltr`（见 §7）。
- 文档标题：根路由挂载 `DocumentTitle`，从 `useMatches()` 取最深的 `staticData.titleKey`，用 `t()` 生成 `${title} · ${t('common.appName')}`；语言变化时通过 `useTranslation()` 重新渲染并更新 `document.title`。`index.html` 只保留中性 fallback 标题 `rs-admin`，移除上游英文 description/OG/Twitter 文案；不引入 `react-helmet-async`。

### 5. key 命名约定

- UI 文案 key 使用 lowerCamelCase 的语义路径，例如 `nav.users`、`auth.signIn.title`、`settings.account.language`、`roles.builtin.super_admin`、`common.actions.save`。
- 后端稳定 code 的 key 保留 snake_case 原文，便于与 API 契约逐字对应，例如 `errors.auth.invalid_credentials`、`errors.role.role_name_taken`、`validation.invalid_format`、`audit.actions.authn_login_success`。
- 顶层错误统一 `errors.<domain>.<code>`，domain 为 `auth`、`user`、`role`、`audit`、`common`；字段错误统一 `validation.<code>`。共享 code 优先 `errors.<domain>.<code>`，缺失时回退 `errors.common.<code>`，再回退 `errors.common.unexpected`。
- 内置角色名称用 `roles.builtin.<code>`（`super_admin`/`admin`/`user`），旁边显示稳定 code chip；自定义角色名称按用户输入原样展示，不翻译。
- 权限组/权限项使用后端 `display_name_zh` / `display_name_en`，按当前 locale 选择；缺失或为空时回退原始 permission key（如 `users.read`），不在前端另建权限名称表。
- 审计日志的 `action`、`target_type`、`severity`、`result` 由前端静态目录本地化：`audit.actions.<action>`、`audit.targetTypes.<type>`、`audit.severity.<value>`、`audit.results.<value>`；未知 action/target_type 显示原始 code，未知 severity/result 同样回退原始值。`actor_label` 为快照，`anonymous`/`cli`/`system` 用 `audit.actors.*`，其他显示原值。
- 插值只用命名参数，如 `{{count}}`、`{{seconds}}`、`{{requestId}}`、`{{max}}`；参数名 lowerCamelCase，值由 React 转义，不使用 HTML 片段。
- 复数 key 使用 i18next 后缀；禁止把 `count` 拼进句子后手工处理单复数。
- key 只能静态引用；错误码/动作目录用 `Record<Code, TranslationKey>` 映射，未知 code 走 fallback。不得由后端字符串直接拼出 key。

### 6. 后端错误文案的本地化边界

- 后端（ADR-0004）只返回稳定 `code`、英文 `title`/`detail`、`errors[].pointer/code/params` 与 `request_id`；不按 `Accept-Language` 变文案。前端是文案归属方。
- 前端同一套映射服务于登录、改密、重新认证、Users、Roles、Audit 与全局错误：
  - 顶层错误：`translateApiError()` 读取 `ApiError.code`，经静态映射选择 `errors.<domain>.<code>` → `errors.common.<code>` → `errors.common.unexpected`。`title`/`detail` 不作为主要文案，仅在开发 console 中保留诊断。
  - 字段错误：`errors[].pointer` 映射到 React Hook Form 字段名；`errors[].code` 映射到 `validation.<code>`，`params` 作为命名插值；未知 pointer 归入表单级错误，未知字段 code 回退 `validation.invalid`。若存在字段错误，不再重复展示 `validation_failed` 顶层文案。
  - `request_id`：从 ProblemDetails 或 `X-Request-Id` 读取；5xx/网络错误、认证/权限错误、无字段映射的页面级错误展示可复制 `request_id`；纯字段校验不额外显示。技术细节行保留原始 `code`，不展示原始 `detail`。
  - `Retry-After`：429 读取响应头（秒数），Clamp 为非负整数；缺失/非法回退 60 秒；展示倒计时并禁用提交，归零后恢复；不做自动重试。账号锁定与 IP/全局限流分别使用 `errors.auth.login_locked` / `errors.auth.rate_limited`（登录/重新认证上下文）或 `errors.common.rate_limited`（其他上下文）。
  - `params`：只做文本插值，不注入 HTML；`password_policy_violation` 的 `params.reason` 用静态映射选择 `validation.password.<reason>`，未知 reason 用通用文案。
- 认证错误映射至少覆盖：`invalid_credentials`、`account_disabled`、`temporary_password_expired`、`login_locked`、`rate_limited`、`session_expired`、`refresh_token_invalid`、`refresh_reuse_detected`、`reauth_required`、`invalid_current_password`、`password_change_required`、`password_policy_violation`、`password_unchanged`、`csrf_failed`。
- 角色错误映射至少覆盖：`role_immutable`、`permission_subset_violation`、`role_name_taken`、`role_in_use`、`empty_permissions`、`invalid_permission`、`unknown_role`；用户错误映射至少覆盖：`identifier_taken`、`self_action_forbidden`、`out_of_scope`、`last_super_admin`；审计/通用错误覆盖：`invalid_time_range`、`time_range_too_large`、`validation_failed`、`invalid_json`、`unknown_field`、`unsupported_media_type`、`method_not_allowed`、`payload_too_large`、`invalid_sort`、`page_size_too_large`、`empty_update`、`forbidden`、`not_found`、`internal_error`、`service_unavailable`。
- 前端 Zod 校验消息：schema 做成接受 `t` 的工厂函数（如 `makeSignInSchema(t)`），在组件内按当前 locale 记忆化创建；消息使用 `validation.*`，不硬编码英文。语言切换后 schema 随 `t` 重建，已显示的错误在下次校验/输入时更新。
- 错误码目录以 ADR-0004 §2 + ADR-0005 的 `reauth_required` 为准；新增未知 code 必须回退，不得导致页面崩溃或显示 `title`/`detail`。

### 7. RTL

- v1 界面仅 LTR。`zh-CN` 与 `en-US` 都映射为 `ltr`。
- 保留 `DirectionProvider`、Radix `DirectionProvider` 与逻辑 CSS 属性（`ps-*`、`pe-*`、`ms-*`、`me-*`、`text-start/end` 等）；保留上游对 shadcn 组件的 RTL 修改，避免未来重新引入 RTL 时重做。
- 移除 ConfigDrawer/Appearance 中的手动 LTR/RTL 切换；`dir` Cookie 不再作为用户可写偏好。`document.documentElement.dir` 固定为 `ltr`；未来加入 RTL 语言时，locale→dir 映射再启用。
- 同步修订 ADR-0006 §8：Appearance 保留主题/字体本地偏好，不再保留用户可见的方向切换；Account 语言选择只写本地 locale 偏好，文案实现见本 ADR。

### 8. bun 迁移与许可

- `package.json`：
  - `"packageManager": "bun@1.4.2"`（与本地/CI/Docker 固定版本一致）。
  - `"engines": { "node": ">=22.18" }`（orval 8.46 的 Node 下限；Bun 仍需通过兼容性验证）。
  - 提交文本锁文件 `bun.lock`，删除 `pnpm-lock.yaml`；不提交 `node_modules`。
- 脚本：保留上游脚本名（`dev`、`build`、`lint`、`format`、`format:check`、`test`、`test:browser:install` 等），新增 `typecheck`（`tsc --noEmit`）、`check`（lint + format:check + typecheck + test + build 的聚合）与 `i18n:check`（key 对齐检查）。文档/CI 统一 `bun run <script>`、`bunx`，不再出现 `pnpm`/`pnpm dlx`。
- CI（前端部分；完整流水线归「部署、可观测与 CI」）：
  - `oven-sh/setup-bun` 固定 bun `1.4.2`，缓存 Bun 安装缓存；`bun install --frozen-lockfile`。
  - 依次运行 lint、format check、typecheck、i18n key 检查、browser install、test、build。
  - orval 生成客户端后继续 `git diff --exit-code` 防漂移；若 Bun 1.4.2 下 orval 8.46 不兼容，按 ADR-0004/0006 回退 `openapi-typescript` + `openapi-fetch`，不退化为手写业务客户端。
  - Playwright 使用 `bunx playwright install --with-deps chromium`。
- Docker（前端构建阶段；运行时静态托管归「部署、可观测与 CI」）：
  - 构建阶段 `oven/bun:1.4.2`，`bun install --frozen-lockfile` + `bun run build`；`VITE_API_BASE_URL` 作为 build arg 注入；产物交给静态运行时（如 nginx），生产镜像不含 Node/bun。
  - `.dockerignore` 排除 `node_modules`、`dist`、`.git` 与本地缓存；CSP/SPA fallback 由部署票据配置。
- 许可：保留上游 `LICENSE` 全文；新增 `THIRD-PARTY-NOTICES.md`（至少列出 shadcn-admin、shadcn/ui、Radix UI、Tailwind CSS、Lucide 等直接依赖的许可证与版权行）；README 增加归属说明；删除 Clerk 品牌素材；替换品牌名称/logo 时避免暗示官方关联。

### 9. 测试与验收

- i18n 测试初始化固定 `zh-CN`（产品主语言），使用独立 i18n 实例并在每个测试后重置；少量语言切换、错误映射用例显式跑 `en-US`。
- CI 增加 key 对齐检查：递归比较 zh-CN/en-US 各 namespace 的 key 集合，缺失/多余都失败；开发/测试 `missingKeyHandler` 直接抛错，生产回退 `en-US`。
- 文本易变的断言改用 `data-testid`/ARIA role；只有文案本身是验收入口时才断言具体字符串。更新上游依赖英文字符串的测试（登录表单、config-drawer、sign-out 等）。
- 至少覆盖：语言解析与 Cookie 持久化、切换后 `<html lang>`/标题/文案更新、错误码映射（认证/角色/用户/通用）、未知 code 回退、`Retry-After` 倒计时、字段错误 pointer 映射、内置角色名称与 code chip、权限显示名回退、审计 action/target_type 回退。
- 不在本票据引入 E2E；真实后端联调与部署验证由「部署、可观测与 CI」承担。
- 验收点：
  - 裁剪后路由树、侧栏、命令菜单、profile 菜单不存在已删除页面/死链；Dashboard/Users 无假数据。
  - 默认 zh-CN；切换 en-US 后菜单、表单、错误、日期、数字、文档标题全部更新；刷新后保持；`<html lang>` 正确；无手动 RTL 入口。
  - 所有保留页面无硬编码用户文案；key 对齐检查通过；缺 key 在开发/测试直接失败。
  - 认证、角色、用户、审计错误按稳定 code 显示本地化文案；429 倒计时；`request_id` 可复制；未知 code 有安全回退。
  - 全新环境 `bun install --frozen-lockfile`、`bun run check`、`bun run build` 通过；CI/Docker 不再依赖 pnpm；许可与第三方声明齐全。
  - `/403`、`/change-password`、`/roles`、`/audit-logs` 按 ADR-0006/0002/0004 的权限与守卫工作；受限会话只能先改密。

## Consequences

- **正面**：页面面与导航一次收敛，删除 demo/Clerk 后死链与假数据风险显著降低；i18next 的类型化 key 与静态资源让文案缺失在开发期暴露；错误码映射把后端契约与前端文案解耦，新增语言/文案不需改后端；保留 RTL 基础设施与逻辑 CSS，未来加 RTL 语言成本低；bun 统一本地/CI/Docker，减少工具链分叉；MIT 归属明确。
- **负面**：两种语言资源随主包加载，包体略增；`react-i18next` 的 key 类型与 namespace 配置有一定学习成本；`bun` 与 orval 8.46 的兼容性仍需在实现首日验证；移除 Faker/演示数据后 Users/Dashboard 在「Users 页面真实 CRUD 集成」完成前只是空壳；LTR-only 意味着上游 RTL 组件改动暂时没有用户可见回报，但保留为未来能力。
- **复查条件**：包体或首屏因 i18n 资源显著变大 → 评估 en-US 按需加载；orval 在 bun 下不兼容 → 按 ADR-0004/0006 回退并记录；Users/Dashboard 空壳影响交付节奏 → 调整实现顺序而非恢复假数据；需要 RTL 语言 → 重新启用方向映射与手动切换并补齐物理 CSS 检查；后端引入服务端本地化 → 重新评估文案归属与缓存。

## Considered Options

- **`react-intl`（ICU MessageFormat）**：格式能力强，但团队需采用新语法，且与稳定错误 code 的映射没有额外收益；不选。
- **单一 `translation.json`**：文件简单，但页面/错误/校验文案混在一起，审查与按功能拆分困难；不选。
- **en-US 懒加载**：减小首包，但引入异步竞态、FOUC 与额外 CSP/网络依赖；v1 不选。
- **始终默认 zh-CN、忽略浏览器语言**：可预测，但英文浏览器用户首次进入体验差；不选，保留“本地偏好 → 浏览器语言 → zh-CN”。
- **localStorage 存语言**：可行，但主题/方向已用 Cookie，Cookie 可在渲染前同步读取；不选。
- **保留手动 RTL 开关**：两种支持语言都是 LTR，手动切换只增加 QA 面；不选，保留基础设施但不在 UI 暴露。
- **彻底移除 RTL 基础设施**：会与上游 RTL 定制组件分叉，未来恢复成本高；不选。
- **保留 Dashboard 演示图表/Faker/TeamSwitcher**：会继续显示假业务数据与不存在的多团队概念，误导验收；不选。
- **错误统一扁平 `errors.<code>`**：简单但无法表达角色/认证等域内不同文案；不选，采用 `errors.<domain>.<code>` + 通用回退。
- **后端按 `Accept-Language` 本地化**：前端简单，但服务端需维护文案、测试矩阵扩大，且与 ADR-0004 冲突；不选。
- **继续使用 pnpm**：与既定技术栈和 CI/Docker 统一目标冲突；不选。
- **保留 `recharts` 但把图表标成占位**：仍会把演示数据带进生产壳；不选，占位壳不渲染假图表。

## References

- [前端裁剪与中英 i18n](https://github.com/nasa-du/rs-admin-ds/issues/13)
- [前端认证集成与路由守卫](https://github.com/nasa-du/rs-admin-ds/issues/12)
- [角色管理 API 与 UI](https://github.com/nasa-du/rs-admin-ds/issues/10)
- [审计日志查询 UI](https://github.com/nasa-du/rs-admin-ds/issues/11)
- [Users 页面真实 CRUD 集成](https://github.com/nasa-du/rs-admin-ds/issues/14)
- [部署、可观测与 CI（含 PG18 升级预留、容量/SLO）](https://github.com/nasa-du/rs-admin-ds/issues/15)
- [规格整合与交付](https://github.com/nasa-du/rs-admin-ds/issues/16)
- [ADR-0002: RBAC 权限模型与授权规则](./0002-rbac-permission-model.md)
- [ADR-0004: API 契约与数据模型](./0004-api-and-data-contract.md)
- [ADR-0005: 安全基线](./0005-security-baseline.md)
- [ADR-0006: 前端认证集成与路由守卫](./0006-frontend-auth-integration.md)
- `docs/research/shadcn-admin-audit.md`（本地 `research/shadcn-admin-audit` 分支，commit `34f5c89`）
