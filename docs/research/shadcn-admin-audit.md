# shadcn-admin 集成审计

> 研究 ticket: wayfinder #4「shadcn-admin 集成审计」
>
> 审计对象: [satnaing/shadcn-admin](https://github.com/satnaing/shadcn-admin)
>
> 审计快照: `main` @ `e16c87f213a5ba5e45964e9b67c792105ec74d26`（2026-06-11 17:08:35 +0700，`package.json` 版本 `2.2.1`）
>
> 方法: 对公开仓库做 shallow clone（`/tmp/shadcn-admin-audit`），静态阅读源码与配置；未运行应用、未安装依赖。文中所有路径均相对上游仓库根目录，raw URL 见文末「来源索引」。

## 0. 结论摘要

- 上游是一个 **纯前端 UI 模板**，不是可直接上线的 starter。README 明确写了 "This is not a starter project (template) though."；认证只是演示级 mock，不存在任何真实 API 调用。
- **没有任何 route guard**。`_authenticated` 布局路由只渲染布局，不检查 token；未登录访问 `/`、`/users`、`/settings` 都能直接打开。
- 登录是**纯客户端 mock**：任意通过格式校验的邮箱/密码，等待 2 秒后写入一个硬编码的 mock user 和字符串 token。token 存在可被 JS 读取的 cookie 里，user 不持久化，`exp` 从不校验。
- **TanStack Query 与 Axios 只有“脚手架”**：QueryClient 配置齐全，但全仓库没有任何 `useQuery` / `useMutation` / API 模块；Axios 只被用来判断 `AxiosError` 类型。API 层需要从零建立。
- **i18n 完全缺失**：没有 i18n 依赖、没有 locale context，所有 UI 文案硬编码英文。RTL 支持是真的（`DirectionProvider` + Radix Direction + 逻辑 CSS 属性），但语言切换与文本方向是两套未打通的东西。
- 裁掉 tasks/apps/chats/help-center/Clerk/多余 auth/error demo 后，保留面是清晰的：dashboard、users、sign-in、settings 五子页、layout、401/404/500/503。删除是低耦合的，Clerk 官方注释也说明可整目录移除。
- 上游为 **MIT License, Copyright (c) 2024 Sat Naing**。分发衍生作品必须保留 copyright 与 permission notice；建议额外维护 `THIRD-PARTY-NOTICES` 与 README 归属说明。
- 迁移到 bun、接入真实 JWT API、接入 zh-CN i18n 都可行，但都需要替换现有 mock 层，而不是“改配置”。

---

## 1. Toolchain 与工程配置

### 1.1 核心版本（`package.json`）

| 类别 | 依赖 | 版本 |
| --- | --- | --- |
| UI | `react` / `react-dom` | `^19.2.5` |
| 构建 | `vite` | `^8.0.8` |
| 路由 | `@tanstack/react-router` | `^1.168.22` |
| 路由插件 | `@tanstack/router-plugin` | `^1.167.22` |
| 服务端状态 | `@tanstack/react-query` | `^5.99.0` |
| 表格 | `@tanstack/react-table` | `^8.21.3` |
| 语言 | `typescript` | `~6.0.3` |
| 样式 | `tailwindcss` / `@tailwindcss/vite` | `^4.2.2` |
| 状态 | `zustand` | `^5.0.12` |
| 表单 | `react-hook-form` + `zod` | `^7.72.1` / `^4.3.6` |
| HTTP | `axios` | `^1.15.0` |
| 认证 | `@clerk/react` | `^6.4.3` |
| 测试 | `vitest` + `@vitest/browser-playwright` | `^4.1.4` |
| 假数据 | `@faker-js/faker`（devDependency） | `^10.4.0` |
| 图标/图表 | `lucide-react` / `recharts` | `^1.8.0` / `^3.8.1` |
| 其他 | `sonner`, `date-fns`, `cmdk`, `input-otp`, `react-day-picker` | 见 `package.json` |

关键点：

- Vite 配置使用 `@vitejs/plugin-react`、`@tailwindcss/vite`、`tanstackRouter({ target: 'react', autoCodeSplitting: true })`，并设置 `@` → `./src` 别名。路由是 **file-based**，`src/routeTree.gen.ts` 由插件生成并提交在仓库中。
- `components.json` 使用 shadcn CLI 配置：style `new-york`、baseColor `slate`、cssVariables、iconLibrary `lucide`、Tailwind v4（`config` 为空、CSS 入口 `src/styles/index.css`）。
- 构建脚本是 `tsc -b && vite build`；`netlify.toml` 只有一条 SPA fallback `/* -> /index.html 200`。
- `index.html` 固定 `<html lang="en">`，标题/描述硬编码；Google Fonts 加载 Inter/Manrope。

### 1.2 包管理器与 lockfile

- 上游使用 **pnpm**：仓库根有 `pnpm-lock.yaml`（约 231 KB）；README 与 `.github/CONTRIBUTING.md` 都写 `pnpm install` / `pnpm run dev`；CI（`.github/workflows/ci.yml`）使用 `npm install -g pnpm` + `pnpm install --frozen-lockfile`，Node 版本 20。
- `package.json` **没有 `packageManager` 字段，也没有 `engines`**。
- 迁移到 bun 的影响面（本 ticket 的既定方向）：
  - 删除 `pnpm-lock.yaml`，执行 `bun install` 生成 `bun.lock`；
  - 建议补 `"packageManager": "bun@<version>"`（可选但可复现）；
  - 更新 README/CONTRIBUTING/CI 中的 `pnpm` 命令为 `bun` / `bun run` / `bunx`，CI 去掉 `npm install -g pnpm`，改用 `oven-sh/setup-bun` + `bun install --frozen-lockfile`；
  - `package.json` 的 scripts 本身与包管理器解耦，`bun run build/test/lint` 可直接复用；
  - `test:browser:install` 仍需要 Playwright/Chromium 安装。
  - 未实际用 bun 安装/构建验证，存在 Vite 8 / Vitest 4 在目标 bun 版本下的兼容性待验证项。

### 1.3 认证依赖（Clerk）

- 唯一 Clerk 依赖是 `@clerk/react ^6.4.3`，全部使用集中在 `src/routes/clerk/**`（外加 `src/assets/clerk-logo.tsx`、`src/assets/clerk-full-logo.tsx`、`src/components/learn-more.tsx`、`src/components/layout/data/sidebar-data.ts` 的入口）。
- `.env.example` 只有一个变量：`VITE_CLERK_PUBLISHABLE_KEY=`（占位为空）。
- `src/routes/clerk/route.tsx` 的提示明确说明：Clerk integration lives entirely inside `src/routes/clerk`，不用 Clerk 时可以安全删除该目录与 `@clerk/react` 依赖。实际 grep 结果也确认 `@clerk/react` 没有出现在 clerk 路由目录之外。
- Clerk 的 `/clerk/sign-in`、`/clerk/sign-up`、`/clerk/user-management` 是演示页；`user-management.tsx` 里有一段运行时 `isSignedIn` 判断，但它只保护这个演示页，不构成应用级 guard。

### 1.4 i18n 依赖

- **无**。`package.json` 中没有 `i18next`、`react-i18next`、`react-intl`、`@formatjs/*`、`next-intl` 等依赖；源码中也没有 i18n/locale 模块。详见第 6 节。

### 1.5 测试与质量

- `vitest` 以 **browser mode** 运行（Playwright chromium，headless），coverage 排除 `src/components/ui/**`、`src/assets/**`、`src/routeTree.gen.ts`、`src/test-utils/**`、`src/routes/**`。
- 仓库内约有 21 个 `*.test.*` 文件，覆盖 `auth-store`、`user-auth-form`、`config-drawer`、`sign-out-dialog`、`password-input`、`confirm-dialog`、`users-*` 对话框、`tasks-*` 对话框、`handle-server-error`、`cookies`、`use-table-url-state` 等。
- ESLint 10 + Prettier 3（含 Tailwind class 排序插件）+ `knip`；CI 会跑 lint、format check、browser install、test、build（knip 在 CI 中被注释）。

---

## 2. 路由与页面清单

### 2.1 路由结构

上游用 TanStack Router 的 **file-based routing**：

- `src/routes/__root.tsx` 是根路由：`NavigationProgress`、`Outlet`、`Toaster`、devtools；`notFoundComponent: NotFoundError`、`errorComponent: GeneralError`。
- `src/routes/_authenticated/route.tsx` 是 pathless layout route，仅 `component: AuthenticatedLayout`。
- `src/routes/(auth)` 与 `src/routes/(errors)` 是 pathless group，目录名不出现在 URL 中。例如文件 `(auth)/sign-in.tsx` 的公开 URL 是 `/sign-in`，route id 是 `/(auth)/sign-in`。
- `src/routes/_authenticated/settings/route.tsx` 是 `/settings` 的布局路由，渲染 `Settings`（侧栏 + `Outlet`）；五个子页是它的 children。
- `src/routes/clerk/**` 是独立挂载在 `/clerk` 下的演示区，有自己的 `(auth)` 与 `_authenticated` 布局。
- `routeTree.gen.ts` 已提交，删除/新增路由文件后需要让 Vite 插件重新生成（`vite dev/build` 时自动完成）；不要手改生成文件。

### 2.2 完整路由/页面清单与去留

| URL | route id / 文件 | 页面/feature | 决策 |
| --- | --- | --- | --- |
| `/` | `/_authenticated/` — `src/routes/_authenticated/index.tsx` | Dashboard | **保留** |
| `/users/` | `/_authenticated/users/` — `src/routes/_authenticated/users/index.tsx` | Users | **保留** |
| `/settings` | `/_authenticated/settings` — `src/routes/_authenticated/settings/route.tsx` | Settings 布局 | **保留** |
| `/settings/` | `/_authenticated/settings/` — `settings/index.tsx` | Profile | **保留** |
| `/settings/account` | `/_authenticated/settings/account` | Account | **保留** |
| `/settings/appearance` | `/_authenticated/settings/appearance` | Appearance | **保留** |
| `/settings/notifications` | `/_authenticated/settings/notifications` | Notifications | **保留** |
| `/settings/display` | `/_authenticated/settings/display` | Display | **保留** |
| `/sign-in` | `/(auth)/sign-in` — `src/routes/(auth)/sign-in.tsx` | 登录 | **保留** |
| `/sign-in-2` | `/(auth)/sign-in-2` | 双栏登录演示 | **删除** |
| `/sign-up` | `/(auth)/sign-up` | 注册演示 | **删除** |
| `/forgot-password` | `/(auth)/forgot-password` | 忘记密码演示 | **删除** |
| `/otp` | `/(auth)/otp` | OTP 演示 | **删除** |
| `/401` | `/(errors)/401` | Unauthorized 错误页 | **保留** |
| `/403` | `/(errors)/403` | Forbidden 错误页 | **删除**（不在保留范围；对应 API 403 处理本来也被注释） |
| `/404` | `/(errors)/404` | Not Found | **保留** |
| `/500` | `/(errors)/500` | General Error | **保留** |
| `/503` | `/(errors)/503` | Maintenance | **保留** |
| `/tasks/` | `/_authenticated/tasks/` | Tasks 演示 | **删除** |
| `/apps/` | `/_authenticated/apps/` | Apps 演示 | **删除** |
| `/chats/` | `/_authenticated/chats/` | Chats 演示 | **删除** |
| `/help-center/` | `/_authenticated/help-center/` | ComingSoon 占位 | **删除** |
| `/errors/$error` | `/_authenticated/errors/$error` | 错误页 demo 容器（`unauthorized`/`forbidden`/`not-found`/`internal-server-error`/`maintenance-error`） | **删除** |
| `/clerk` | `clerk/route.tsx` | ClerkProvider 布局 | **删除** |
| `/clerk/sign-in` | `clerk/(auth)/sign-in.tsx` | Clerk 登录 | **删除** |
| `/clerk/sign-up` | `clerk/(auth)/sign-up.tsx` | Clerk 注册 | **删除** |
| `/clerk/user-management` | `clerk/_authenticated/user-management.tsx` | Clerk 用户管理演示 | **删除** |
| — | `src/routes/__root.tsx` | Root layout / 全局错误边界 | **保留** |
| — | `src/features/auth/auth-layout.tsx` | Auth 居中布局（组件，不是 route） | **保留**（sign-in 使用） |
| — | `src/components/layout/authenticated-layout.tsx` | 登录后布局 | **保留** |
| — | `clerk/(auth)/route.tsx`、`clerk/_authenticated/route.tsx` | Clerk 子布局 | **删除** |

### 2.3 保留部分的连带文件

保留页面依赖的核心文件（不要随 demo 一起删）：

- 布局：`src/components/layout/{authenticated-layout,app-sidebar,app-title,header,main,nav-group,nav-user,team-switcher,top-nav,types}.tsx`、`src/components/layout/data/sidebar-data.ts`。
- 通用表格：`src/components/data-table/**`、`src/hooks/use-table-url-state.ts`。
- 通用 UI：`src/components/ui/**`（注意其中部分组件可能随 demo 变成未使用，见下）。
- 通用组件：`config-drawer`、`profile-dropdown`、`search`、`theme-switch`、`sign-out-dialog`、`confirm-dialog`、`password-input`、`date-picker`、`select-dropdown`、`long-text`、`navigation-progress`、`skip-to-main`。
- providers：`theme-provider`、`font-provider`、`direction-provider`、`layout-provider`、`search-provider`。
- 错误页组件：`unauthorized-error`、`not-found-error`、`general-error`、`maintenance-error`。
- `src/components/command-menu.tsx` 直接消费 `sidebarData`，侧栏裁剪后会自动同步，无需单独改链接。

### 2.4 删除清单（文件级）

路由：

- `src/routes/_authenticated/tasks/index.tsx`
- `src/routes/_authenticated/apps/index.tsx`
- `src/routes/_authenticated/chats/index.tsx`
- `src/routes/_authenticated/help-center/index.tsx`
- `src/routes/_authenticated/errors/$error.tsx`
- `src/routes/(auth)/sign-in-2.tsx`
- `src/routes/(auth)/sign-up.tsx`
- `src/routes/(auth)/forgot-password.tsx`
- `src/routes/(auth)/otp.tsx`
- `src/routes/(errors)/403.tsx`
- 整个 `src/routes/clerk/**`

Feature 目录/文件：

- `src/features/tasks/**`
- `src/features/apps/**`（含 `src/assets/brand-icons/**` 的 Apps 使用）
- `src/features/chats/**`
- `src/features/auth/sign-up/**`
- `src/features/auth/forgot-password/**`
- `src/features/auth/otp/**`
- `src/features/auth/sign-in/sign-in-2.tsx` 及 `src/features/auth/sign-in/assets/*.png`
- `src/features/errors/forbidden.tsx`
- `src/components/coming-soon.tsx`（仅 help-center 使用）
- `src/components/learn-more.tsx`（仅 Clerk 演示使用）
- `src/assets/clerk-logo.tsx`、`src/assets/clerk-full-logo.tsx`
- 可选：`src/assets/brand-icons/**`。sign-in 页目前用 `IconGithub` / `IconFacebook` 渲染两个**无 onClick 的装饰按钮**；如果确定不做 OAuth，可一并删除整个 brand-icons 与两个按钮。

组件/依赖：

- 可删除依赖：`@clerk/react`；`input-otp`（只被 OTP demo 使用）；`@faker-js/faker`（当前被 `src/features/users/data/users.ts` 与 `src/features/tasks/data/tasks.ts` 使用，tasks 删除且 users 接入真实 API 后可删）。
- `src/components/ui/input-otp.tsx` 可删；`src/components/ui/radio-group.tsx` 仍被 settings 使用，不能删。
- 其余依赖不要凭 demo 猜测删除：`recharts` 被 dashboard 使用，`date-fns` 被 `date-picker` 使用，`cmdk` 被 command-menu 与 account 语言选择使用，`react-day-picker` 被 calendar 使用。

侧栏/导航配置：

- `src/components/layout/data/sidebar-data.ts` 需要裁剪：
  - 删除 General 组中的 Tasks/Apps/Chats、`Secured by Clerk` 整组；
  - 删除 Pages/Auth 整组（Sign In 2 Col/Sign Up/Forgot Password/OTP）；
  - 删除 Pages/Errors 整组（`/errors/*` demo）；
  - 删除 Other 中的 Help Center；
  - 保留 Dashboard、Users、Settings（五个子项）。
- `sidebarData.user` 是硬编码的 `satnaing` / `satnaingdev@gmail.com`，`sidebarData.teams` 是三个演示团队。接入真实登录后应改为从 auth store/API 读取，或移除 team switcher。
- `src/components/profile-dropdown.tsx` 也硬编码 `satnaing` / 邮箱与 `/avatars/01.png`；需要接入真实用户或简化。
- 删除后需要清理 `sidebar-data.ts` 中不再使用的 icon imports（`ListTodo`、`Package`、`MessagesSquare`、`HelpCircle`、`Bug`、`Construction`、`FileX`、`ServerOff`、`Lock`、`UserX`、`ShieldCheck`、`ClerkLogo` 等），否则 lint/knip 会报未使用。
- `src/components/command-menu.tsx` 遍历 `sidebarData.navGroups`，删除后自动生效；但其中 `Theme` 命令（Light/Dark/System）需要保留。

链接清理点：

- `src/features/auth/sign-in/index.tsx` 仍有 `Sign Up` 链接（指向 `/sign-up`）和 `/terms`、`/privacy` 两个不存在的锚点；裁剪后应删除或替换。
- `src/features/auth/sign-in/components/user-auth-form.tsx` 有 `Forgot password?` 链接（指向 `/forgot-password`）与 GitHub/Facebook 按钮；裁剪后应删除或替换为真实能力。
- `src/components/layout/nav-user.tsx` 的 `Billing` 指向 `/settings`（不是独立计费页），`Upgrade to Pro` 无行为；按产品需要清理。
- `src/components/profile-dropdown.tsx` 的 Billing/Settings 都指向 `/settings`，New Team 无行为。

### 2.5 Dashboard 现状（保留页面，但要替换数据）

文件：`src/features/dashboard/index.tsx` 及 `src/features/dashboard/components/**`。

- 页面结构：Header（`TopNav` + Search + ThemeSwitch + ConfigDrawer + ProfileDropdown）、标题与 Download 按钮、`Tabs`（Overview / Analytics / Reports(disabled) / Notifications(disabled)）。
- 指标卡：Total Revenue、Subscriptions、Sales、Active Now 都是硬编码数字与环比文案。
- Overview：`recharts` 柱状图，数据在模块加载时用 `Math.random()` 生成 12 个月数据，每次加载不同。
- Recent Sales：5 条硬编码销售记录，头像引用 `/avatars/01.png` 至 `/avatars/05.png`，但 `public/` 下没有 `avatars/` 目录（已验证只有 `public/images/*`），因此这些头像会 404。
- Analytics：`analytics.tsx` / `analytics-chart.tsx` 也是静态演示数据。
- TopNav 的 Customers/Products/Settings 三个链接是 disabled 状态。
- 结论：Dashboard 属于保留范围，但和 users 一样是 demo 数据，接入真实系统时应改为 API 驱动的 widgets，或先保留为静态“欢迎页”并明确标注。

---

## 3. Mock auth 实现

### 3.1 Auth store 与 token cookie

文件：`src/stores/auth-store.ts`

```ts
const ACCESS_TOKEN = 'thisisjustarandomstring'
```

- 这个常量是 **cookie 名**，不是密钥（值本身公开且无安全含义）。
- Store 初始化时同步读取 cookie：`getCookie(ACCESS_TOKEN)`，并 `JSON.parse` 后作为 `accessToken`。如果 cookie 内容不是合法 JSON，模块初始化会抛异常。
- `setAccessToken` 把 token `JSON.stringify` 后写入 cookie；`reset()` / `resetAccessToken()` 删除 cookie。
- cookie 工具在 `src/lib/cookies.ts`：默认 `max-age=60*60*24*7`（7 天），写入格式为 `name=value; path=/; max-age=...`。**没有 `Secure`、`SameSite`、`HttpOnly`**，且本身是 `document.cookie` 实现，天然无法 HttpOnly。
- User 的初始值是 `user: null`，**只有 token 被持久化，user 不持久化**。刷新页面后 store 能恢复 token，但 user 永远回到 null；由于没有 guard，UI 仍会正常渲染，侧栏/头像仍是硬编码数据。
- `AuthUser` 形状为 `{ accountNo: string; email: string; role: string[]; exp: number }`；`exp` 在整个仓库中没有被读取或校验。

### 3.2 登录表单行为

文件：`src/features/auth/sign-in/components/user-auth-form.tsx`

- Zod schema：`email` 必须是合法邮箱；`password` 至少 1 位且至少 7 位。
- `onSubmit` 不做任何网络请求：
  1. `toast.promise(sleep(2000), ...)` 模拟 2 秒等待；
  2. 成功回调里构造 mock user：`{ accountNo: 'ACC001', email: data.email, role: ['user'], exp: Date.now() + 24h }`；
  3. `auth.setUser(mockUser)`；`auth.setAccessToken('mock-access-token')`；
  4. `navigate({ to: redirectTo || '/', replace: true })`；
  5. toast `Welcome back, <email>!`。
- 也就是说 **任意格式合法的邮箱 + 任意 7 位以上密码都能“登录成功”**。
- `redirectTo` 来自 `src/routes/(auth)/sign-in.tsx` 的 `validateSearch`：`redirect: z.string().optional()`，没有白名单/同源校验。接入真实系统时应限制为站内路径。
- 页脚 Terms/Privacy 是 `href='/terms'`、`href='/privacy'`，项目里没有这两个路由。
- 社交登录按钮没有 `onClick`，纯装饰。
- 对应测试 `src/features/auth/sign-in/components/user-auth-form.test.tsx` 明确断言了 `setAccessToken('mock-access-token')` 与跳转行为，可作为改造时的回归基线。

### 3.3 缺失的 route guard

- `src/routes/_authenticated/route.tsx` 全文只有：

  ```ts
  export const Route = createFileRoute('/_authenticated')({
    component: AuthenticatedLayout,
  })
  ```

- 全仓库 grep `beforeLoad`、`redirect(`、`isAuthenticated`、`accessToken` 的结果显示：**没有任何路由在进入前检查 token**。`accessToken` 只出现在 store 与测试中。
- 唯一的“认证判断”是 Clerk 演示页 `src/routes/clerk/_authenticated/user-management.tsx` 里的 `const { isLoaded, isSignedIn } = useAuth()`，与本 ticket 的保留范围无关。
- 因此裁剪后如果没有新增 guard，未登录用户可以直接访问 `/`、`/users`、`/settings/*`。这是集成时优先级最高的安全问题。
- 现有的“401 处理”只发生在某次 API 请求真的返回 401 之后（见 3.5），不可能替代路由级 guard。

### 3.4 退出登录

文件：`src/components/sign-out-dialog.tsx`、`src/components/layout/nav-user.tsx`、`src/components/profile-dropdown.tsx`

- 点击菜单里的 Sign out 会打开 `ConfirmDialog`。
- 确认后执行：
  - `auth.reset()`：删除 token cookie、清空 user/accessToken；
  - `navigate({ to: '/sign-in', search: { redirect: location.href }, replace: true })`。
- 由于 `location.href` 可能是完整 URL，接入真实路由时应改为站内 path 并做合法性校验。
- 退出不会调用后端撤销 refresh token（目前没有后端）。

### 3.5 全局 401/500 处理

文件：`src/main.tsx`（第 51-72 行）

```ts
queryCache: new QueryCache({
  onError: (error) => {
    if (error instanceof AxiosError) {
      if (error.response?.status === 401) {
        toast.error('Session expired!')
        useAuthStore.getState().auth.reset()
        const redirect = `${router.history.location.href}`
        router.navigate({ to: '/sign-in', search: { redirect } })
      }
      if (error.response?.status === 500) {
        toast.error('Internal Server Error!')
        if (import.meta.env.PROD) router.navigate({ to: '/500' })
      }
      if (error.response?.status === 403) {
        // router.navigate("/forbidden", { replace: true });
      }
    }
  },
})
```

- 401 会清空 auth 并带 `redirect` 跳转 `/sign-in`；500 仅在 production 跳 `/500`；403 分支被注释。
- 这段逻辑依赖 `AxiosError`；如果接入 `fetch` 或别的 HTTP client，需要重写。
- 目前没有任何实际请求，所以这段逻辑从未被触发。

### 3.6 接入真实 JWT 时必须补的能力（审计结论）

1. **路由级 guard**：在 `_authenticated/route.tsx` 的 `beforeLoad` 中检查 token/会话（或调用 `/me`），未认证时 `throw redirect({ to: '/sign-in', search: { redirect: location.href } })`。
2. **登录改为真实请求**：`POST /auth/login`，保存 access token（推荐内存 + HttpOnly refresh cookie，或后端下发的 HttpOnly cookie），不要继续把 token 放进可读 cookie。
3. **用户信息**：登录响应或 `/me` 返回真实 user；持久化/重新拉取，替换 `sidebarData.user`、`profile-dropdown`、`nav-user` 中的硬编码。
4. **过期与刷新**：使用 `exp` 或后端返回的 `expires_in`；实现 refresh 或 401 重试一次再登出，避免 401 时无条件清空。
5. **redirect 校验**：只允许站内路径，防止开放重定向。
6. **401 处理去重**：QueryCache 与 axios interceptor 同时处理容易重复 toast/跳转，建议收敛到 api client 一处。
7. **登录页清理**：移除 sign-up/forgot-password/OTP/社交按钮/无效 Terms 链接，或补上对应真实流程。

---

## 4. Users 页面

### 4.1 数据源：Faker

文件：`src/features/users/data/users.ts`

```ts
import { faker } from '@faker-js/faker'
faker.seed(67890)

export const users = Array.from({ length: 500 }, () => {
  // id / firstName / lastName / username / email / phoneNumber
  // status: active | inactive | invited | suspended
  // role: superadmin | admin | cashier | manager
  // createdAt / updatedAt
})
```

- 500 条数据在**模块加载时同步生成**，固定 seed 67890；每次整页加载重新生成。
- `createdAt` / `updatedAt` 是 `Date` 对象；`id` 是 UUID 字符串。
- `Users` 页面直接 `import { users }` 并作为 `data` 传给表格，没有 loading/error/empty 状态，也没有请求层。

### 4.2 类型与 schema

- `src/features/users/data/schema.ts` 用 Zod 定义 `UserStatus`、`UserRole` 和 `User`；`createdAt/updatedAt` 使用 `z.coerce.date()`。
- `User` 字段：`id, firstName, lastName, username, email, phoneNumber, status, role, createdAt, updatedAt`。**没有 avatar、accountNo、password 等字段**。
- `src/features/users/data/data.ts` 定义状态徽章样式 `callTypes` 与四种角色 `roles`（含 icon）。角色枚举与真实后端 RBAC 需要对齐。
- `src/tanstack-table.d.ts` 扩展了 column meta：`className`、`tdClassName`、`thClassName`。

### 4.3 表格列、筛选、排序、分页

文件：`src/features/users/components/users-columns.tsx`、`users-table.tsx`

列：

| 列 | 说明 |
| --- | --- |
| `select` | 多选 checkbox；sticky 首列 |
| `username` | `LongText`，sticky，`enableHiding: false` |
| `fullName` | 由 `firstName + lastName` 拼接展示 |
| `email` | `nowrap` |
| `phoneNumber` | 不排序 |
| `status` | Badge，颜色来自 `callTypes`；faceted filter；不排序 |
| `role` | icon + 文本；faceted filter；不排序 |
| `actions` | 行内菜单：Edit / Delete |

筛选/分页：

- URL search 由 `src/routes/_authenticated/users/index.tsx` 校验：
  - `page`（1-based，默认 1）
  - `pageSize`（默认 10）
  - `status: ('active' | 'inactive' | 'invited' | 'suspended')[]`
  - `role: (...)[]`
  - `username: string`
- `useTableUrlState`（`src/hooks/use-table-url-state.ts`）把 URL search 映射为 TanStack Table 的 `pagination` / `columnFilters`：
  - `pageIndex = page - 1`；
  - 修改筛选会重置 `page`；
  - 分页/筛选变更通过 `navigate({ search })` 写回 URL；
  - `ensurePageInRange` 在删除/筛选后把越界页码重置。
- 表格使用 `getPaginationRowModel()` / `getFilteredRowModel()`，即**完全客户端分页与筛选**，500 条数据一次性在前端。
- 工具栏：`DataTableToolbar` 的全局搜索绑定到 `username` 列；facet filter 提供 Status/Role 两组选项。
- 排序：`sorting` 是组件本地 state，不进 URL；username/email 可排序（fullName 列由于没有 accessor，排序实际无数据依据，属于小瑕疵）。

### 4.4 操作与对话框

文件：`src/features/users/components/**`

- `UsersProvider`：用 React Context + `useDialogState` 管理 `open: 'invite' | 'add' | 'edit' | 'delete' | null` 与 `currentRow`。
- `UsersPrimaryButtons`：Invite User / Add User。
- `DataTableRowActions`：Edit / Delete，写入 `currentRow` 后打开对应 dialog。
- `UsersActionDialog`（Add/Edit）：
  - 字段：firstName、lastName、username、email、phoneNumber、role、password、confirmPassword、isEdit；
  - 密码规则：新增时必填、至少 8 位、至少一个小写字母、至少一个数字、与确认密码一致；编辑时密码可为空；
  - 提交只执行 `form.reset()` + `showSubmittedData(values)` + 关闭 dialog，**不调用 API**。
- `UsersInviteDialog`：email、role、可选 description；提交仅 toast。
- `UsersDeleteDialog`：要求输入当前行的 username 才能确认；确认后仅 toast。
- `UsersMultiDeleteDialog`：要求输入 `DELETE`；确认后 `sleep(2000)` 后 toast，并清空选择。
- `DataTableBulkActions`：Invite selected / Activate / Deactivate / Delete；全部是 `sleep(2000)` + toast，无 API。
- `showSubmittedData`（`src/lib/show-submitted-data.tsx`）把提交值 JSON 打印到 sonner toast，是典型的 demo 辅助函数；接入真实 API 后应删除或仅用于调试。

### 4.5 改为真实 API 的变更清单

数据获取：

- 在 `src/features/users/index.tsx` 中用 `useQuery` 替换 `import { users }`；建议新增 `src/features/users/api/`（或 `queries.ts`）封装 `listUsers(params)`。
- 请求参数应复用 URL search：`page`、`pageSize`、`status[]`、`role[]`、`username`（或后端约定的 `q`）。
- 表格改为 **server-side**：
  - `manualPagination: true`、`manualFiltering: true`、`manualSorting: true`；
  - 移除 `getPaginationRowModel()` / `getFilteredRowModel()`（或保留 `getCoreRowModel` + faceted 数据由后端返回）；
  - `pageCount = Math.ceil(total / pageSize)`，`rowCount` 也需设置；
  - 排序状态需要进入 URL search schema（当前 `sorting` 是本地 state，不进 URL）。
- 定义 API DTO → `User` 的映射：后端可能返回 `created_at`/`updated_at`、数字 id、不同枚举值；需要适配层。
- 处理 loading（skeleton）、error（重试/错误页）、empty（"No results."）状态；当前只有静态数据，没有这些分支。
- `DataTableToolbar` 的搜索建议加 debounce，避免每个按键都发请求；`useTableUrlState` 已负责把值写入 URL。

写操作：

- Add：`POST /users`；成功后 `queryClient.invalidateQueries({ queryKey: ['users'] })`，关闭 dialog 应等请求成功。
- Edit：`PATCH/PUT /users/:id`；`currentRow` 作为初始值，提交后 invalidate。
- Delete：`DELETE /users/:id`；确认输入 username 的交互可保留，但成功后要 invalidate。
- Invite：`POST /users/invitations`（或后端约定）；role 枚举对齐。
- Bulk：`POST /users/bulk/status`、`POST /users/bulk/delete`、`POST /users/bulk/invite`（或循环单条，视后端能力）；删除/变更后清空 row selection 并 invalidate。
- 所有 mutation 需要 `isPending` 禁用按钮、错误 toast（可复用 `handleServerError`，但错误契约要改，见第 7 节）。

类型与安全：

- 密码只在 create 时提交；edit 时不应把空密码发给后端（当前 schema 用 `isEdit && !password` 处理）。
- 前端校验只是 UX，后端必须重复校验。
- 角色/操作权限需要前端按当前用户 role 做展示控制（当前没有权限模型）。
- 删除 `@faker-js/faker` 与 `src/features/users/data/users.ts` 后，相关测试 fixture 需要改为固定对象/MSW。
- 用户列表目前每页 10 条、500 条 mock；真实 API 需要确认分页 envelope（如 `{ items, total, page, pageSize }`）与错误结构。
- 头像缺失：`RecentSales`/`ProfileDropdown` 引用 `/avatars/01.png` 等路径，但 `public/` 下只有 `public/images/*`，没有 `public/avatars/*`（上游即如此）。接入真实用户时应使用后端 avatar URL 或占位图。

---

## 5. Settings 五个子页

### 5.1 布局与导航

- `src/routes/_authenticated/settings/route.tsx` → `src/features/settings/index.tsx`：Header + `Main fixed` + 左侧 `SidebarNav` + `<Outlet />`。
- `SidebarNav`（`src/features/settings/components/sidebar-nav.tsx`）在桌面端渲染纵向链接列表，移动端渲染 `<Select>`；当前项用 `pathname` 高亮。
- 每个子页由 `ContentSection`（`src/features/settings/components/content-section.tsx`）包裹：标题、描述、分隔线、可滚动内容区。

### 5.2 五个子页的当前行为

| 子页 | 路由/文件 | 表单内容 | 当前行为 |
| --- | --- | --- | --- |
| Profile | `/settings/` — `src/features/settings/profile/*` | username、email（3 个硬编码选项）、bio、可增删的 URLs | `showSubmittedData(data)`，纯 toast；默认 bio/urls 是硬编码假数据 |
| Account | `/settings/account` — `src/features/settings/account/*` | name、date of birth（DatePicker）、language（静态 9 种语言，含 `zh`） | `showSubmittedData`；language 只是字符串，不触发任何语言切换 |
| Appearance | `/settings/appearance` — `src/features/settings/appearance/*` | font（inter/manrope/system）、theme（light/dark 预览卡） | 真实生效：调用 `FontProvider.setFont` / `ThemeProvider.setTheme`，写入 `font` / `vite-ui-theme` cookie 并更新 `<html>` class；同时仍 toast |
| Notifications | `/settings/notifications` — `src/features/settings/notifications/*` | 通知类型 radio（all/mentions/none）、4 个 email switch（security 禁用）、mobile checkbox | `showSubmittedData`，纯 toast |
| Display | `/settings/display` — `src/features/settings/display/*` | 6 个 sidebar item checkbox（recents/home/applications/desktop/downloads/documents） | `showSubmittedData`，纯 toast；**与实际侧栏显示无关** |

补充：

- `Appearance` 的 theme 表单只支持 light/dark，而 `ThemeProvider` 支持 `system`；ConfigDrawer 可以选 system，两个入口的能力不一致。
- `Account` 的语言列表在 `src/features/settings/account/account-form.tsx` 的 `languages` 常量中，包含 `en/fr/de/es/pt/ru/ja/ko/zh`，但没有 locale provider，选 `zh` 不会改变 UI。
- `Profile` 的 email 选项是 `m@example.com` / `m@google.com` / `m@support.com`，默认值只有 bio/urls；username 没有默认值。
- `Display` 的 `items` 默认 `['recents', 'home']`，但没有任何代码读取它来过滤侧栏；要真正生效需要把选择状态提升到 `LayoutProvider`/`sidebarData` 或用户偏好 API。
- 五个页面的提交函数都直接调用 `showSubmittedData`，没有一个调用 API，也没有 loading/error 状态。

### 5.3 接入真实 API 时的变更

- Profile：加载当前用户资料（`GET /me` 或 `GET /users/:id`），提交 `PATCH /me`；email 选项来自已验证邮箱列表；URLs 字段需要后端数组结构。
- Account：name/dob 提交到用户资料；language 要接入 i18n（见第 6 节），并持久化到用户偏好或 cookie；日期格式需要 locale 化。
- Appearance：theme/font 可以继续本地 cookie（无需后端），也可以持久化到用户偏好；需要统一 system 选项。
- Notifications：定义通知偏好 DTO（type、communication/marketing/social/security、mobile），`GET/PATCH /me/notifications`；security 项仍应只读。
- Display：明确产品语义；若只是控制侧栏显示，建议接入 `LayoutProvider` 并持久化；若需要后端，提交 `PATCH /me/preferences`。
- 所有表单的 Zod message 目前是硬编码英文，i18n 时要改为可翻译的 schema factory 或显示层翻译。

---

## 6. i18n / RTL 现状与接入点

### 6.1 i18n 现状：完全没有

- 无 i18n 依赖（第 1.4 节）。
- 所有文案硬编码在组件中：布局、侧栏（`sidebar-data.ts`）、登录/错误页、users、settings、toast、Zod 错误消息、命令菜单占位符等。
- `index.html` 固定 `<html lang="en">`；标题/描述/OG meta 硬编码英文。
- `index.html` 的 `lang` 不会随任何状态变化；`Account` 的语言选择只是表单值。
- 路由层没有 `head`/document title 管理；TanStack Router 的 head API 未被使用。

### 6.2 RTL 现状：基础设施存在且较完整

- `src/context/direction-provider.tsx`：
  - `Direction = 'ltr' | 'rtl'`，默认 `ltr`；
  - 从 cookie `dir` 读取（1 年有效期）；
  - `useEffect` 设置 `document.documentElement.dir`；
  - 同时用 Radix `DirectionProvider` 包裹 children，使 Radix 组件感知方向。
- `src/components/config-drawer.tsx` 的 `DirConfig` 提供 LTR/RTL 切换与 reset；`config-drawer.test.tsx` 覆盖了方向切换与 `<html dir>`。
- 代码大量使用逻辑 CSS 属性：`inset-s-*`、`inset-e-*`、`ps-*`、`pe-*`、`ms-*`、`me-*`、`text-start/end`，而不是 left/right，因此 RTL 布局基础较好。
- README 明确列出为 RTL 修改过的 shadcn 组件：alert-dialog、calendar、command、dialog、dropdown-menu、select、table、sheet、sidebar、switch；以及 modified components：scroll-area、sonner、separator。用 shadcn CLI 更新这些组件时需要手动合并，避免丢失 RTL 修改。
- Recharts 的 `YAxis direction='ltr'` 是显式设置（dashboard/overview），RTL 下图表仍保持从左到右的数值轴，这是有意为之。
- 结论：**zh-CN 是 LTR，可直接复用现有布局；RTL 机制可以保留，但不要让“语言 = 方向”自动耦合覆盖用户在 ConfigDrawer 中的手动选择。** 建议 locale 默认映射方向，手动选择优先。

### 6.3 建议的 i18n 接入点

依赖与初始化：

- 选型建议：`i18next` + `react-i18next`（生态成熟、支持 plurals/interpolation/namespaces），或 `react-intl`（ICU MessageFormat）。二者都可行；关键是要有一层 `LocaleProvider` 统一处理 `<html lang>`、`dir`、日期/数字格式。
- 新建 `src/i18n/index.ts`（或 `src/lib/i18n.ts`）初始化 i18n，资源按 namespace 拆分，例如 `common`、`auth`、`users`、`settings`、`errors`、`sidebar`。
- 在 `src/main.tsx` 中、`RouterProvider` 外层初始化/包裹 `I18nextProvider`；`StrictMode` 下注意只初始化一次。
- 初始语言：`localStorage`/cookie（可复用 `getCookie`）→ `navigator.language` → fallback `en`；设置页的 language 选择写入同一来源。
- 同步 `<html lang>`（`zh-CN`）和默认 `dir`；保留 `DirectionProvider`，把 locale→dir 作为默认值，手动 `dir` cookie 覆盖。

需要改造的文案位置（按优先级）：

1. `src/components/layout/data/sidebar-data.ts`：当前是模块级常量，改为 `useSidebarData()` + `t('sidebar.dashboard')` 等；`CommandMenu` 也要跟着消费 hook。
2. `src/components/layout/*`、`profile-dropdown`、`nav-user`、`sign-out-dialog`、`config-drawer`：菜单/标题/按钮/aria-label。
3. `src/features/auth/sign-in/**`：表单 label、placeholder、Zod message、toast、页脚。
4. `src/features/users/**`：表头、筛选项、按钮、对话框标题/描述/校验消息、toast、空状态。
5. `src/features/settings/**`：五个子页的标题/描述/字段/选项/按钮。
6. `src/features/errors/**`：401/404/500/503 文案。
7. 全局：`Toaster` 中的消息、`handleServerError` 的 fallback、`search` 的 placeholder、快捷键提示。
8. `index.html` 的 `<title>`/meta 需要用运行时 head 管理（TanStack Router `head` 或 `react-helmet-async` 等价方案）。
9. Zod schema：错误消息在 schema 创建时固化，建议把 schema 做成接受 `t` 的工厂函数，或统一改为错误 code 再在 FormMessage 层翻译。
10. 日期/数字：`date-fns` 的 locale（`date-fns/locale/zh-CN`）用于 `date-picker` 和日期展示；金额/数字用 `Intl.NumberFormat`。
11. 测试：现有 Vitest browser tests 断言英文字符串；引入 i18n 后需要固定测试 locale 或改为断言 `data-testid`/key。

RTL 与 locale 的关系：

- 建议维护映射：`zh-CN → ltr`，`en → ltr`，未来的 `ar`/`he → rtl`；在 locale 切换时更新默认 `dir`，但 ConfigDrawer 的手动选择仍可覆盖并持久化。
- 由于现有逻辑 CSS 属性已经大面积使用，新增中文文案不需要改布局；新增 RTL 语言时才需要检查剩余物理属性（grep `left-|right-|ml-|mr-|pl-|pr-`）。

---

## 7. API / config 约定与 TanStack Query / Axios

### 7.1 现状：只有错误处理脚手架，没有 API 层

- 全仓库没有任何 `useQuery` / `useMutation` / `queryOptions` 调用；`queryClient` 只出现在 `main.tsx` 和 `__root.tsx` 的 context 类型中。
- 没有 `src/api/`、`src/lib/api-client.ts`、`src/services/`、`src/queries/` 之类的模块；没有 axios instance、`baseURL`、interceptor、Authorization header、`withCredentials`。
- Axios 只被 import 了 `AxiosError`：
  - `src/main.tsx`（retry / QueryCache / mutation onError 判断）
  - `src/lib/handle-server-error.ts`（读取 `error.response.data.title`）
  - 以及对应测试。
- 因此所有页面都是“静态数据 + toast”，没有后端契约可继承。

### 7.2 已有的约定（需要在新 API 层中保持或替换）

`QueryClient` 默认值（`src/main.tsx`）：

- queries：
  - retry：开发环境不重试；生产环境超过 3 次停止；401/403 永不重试；
  - `refetchOnWindowFocus: import.meta.env.PROD`；
  - `staleTime: 10 * 1000`。
- mutations：`onError` 调用 `handleServerError(error)`；304 额外 toast `Content not modified!`。
- queryCache：见 3.5 的 401/500/403 处理。
- Router context 已注入 `{ queryClient }`，但没有任何 loader 使用；如果后续用 loader 预取，可以直接 `context.queryClient.ensureQueryData(...)`。

错误契约（`src/lib/handle-server-error.ts`）：

```ts
if (error instanceof AxiosError) {
  const title = error.response?.data?.title
  if (typeof title === 'string' && title.length > 0) errMsg = title
}
```

- 上游期望后端错误体至少有 `{ title: string }`；否则统一 `Something went wrong!`。
- 还特殊处理 `status === 204` 为 `No content.`（注意是 `'status' in error` 的对象判断，不是 Axios response）。
- 接入 Rust/Axum 后端时，要么让错误体满足 `title`，要么修改 `handleServerError` 适配 `{ code, message, details }`。

环境变量：

- `.env.example` 只有 `VITE_CLERK_PUBLISHABLE_KEY=`；`.gitignore` 忽略 `.env`。
- 没有 `VITE_API_BASE_URL` 或类似配置；`src/vite-env.d.ts` 只有 `/// <reference types="vite/client" />`，没有自定义 `ImportMetaEnv`。
- 接入后端时建议新增：
  - `.env.example`：`VITE_API_BASE_URL=http://localhost:3000/api`（或同源 `/api`）；
  - `src/vite-env.d.ts` 声明 `readonly VITE_API_BASE_URL: string`；
  - `src/lib/api-client.ts`：axios instance（`baseURL`、`timeout`、`withCredentials` 或 Bearer interceptor）。

### 7.3 建议建立的项目约定（供后续实现 ticket 使用）

1. **单一 API client**：`src/lib/api-client.ts` 创建 axios instance；请求 interceptor 注入 `Authorization: Bearer <accessToken>`（如果采用内存 token）或依赖 HttpOnly cookie（`withCredentials: true`）；响应 interceptor 统一把后端错误映射为应用错误类型。
2. **鉴权与刷新**：401 时先尝试 refresh 一次；失败再清空 auth 并跳转 `/sign-in`。避免 QueryCache + interceptor 双重跳转。
3. **Query key 规范**：如 `['users', 'list', params]`、`['users', 'detail', id]`、`['me']`；mutation 成功后按前缀 invalidate。
4. **分页 envelope**：统一 `{ items: T[], total: number, page: number, pageSize: number }`，或与 Axum/SQLx 后端约定一致；前端 `useTableUrlState` 的 1-based `page` 映射到后端。
5. **DTO 与 domain 类型分离**：API 返回 snake_case → 前端 `User` camelCase 的 adapter；日期统一 ISO 8601 字符串，进入表格前 `new Date()`/`date-fns` 格式化。
6. **错误展示**：`handleServerError` 适配后端错误体；表单字段错误映射到 `FormMessage`；全局 500 才跳 `/500`。
7. **测试**：Vitest browser mode 下用 MSW 或模块 mock 覆盖 loading/error/success；现有测试是直接 mock store/模块，可延续。
8. **不要保留** `showSubmittedData` 作为生产提交路径；它只适合 demo/调试。

---

## 8. License 与归属义务

- 上游 `LICENSE` 是标准 **MIT License**：`Copyright (c) 2024 Sat Naing`。
- MIT 的核心义务：
  - 在软件副本或“substantial portions”中保留上述 copyright notice 与 permission notice；
  - 允许修改、合并、发布、再许可，但分发衍生作品时必须包含原始 MIT 声明；
  - 软件按 “AS IS” 提供，无担保；
  - MIT 不授予商标权，不得暗示原作者背书。
- 对本项目的实际操作建议：
  1. 保留上游 `LICENSE` 文件（或在合并后的 LICENSE/NOTICE 中完整保留 `Copyright (c) 2024 Sat Naing` 与 MIT 正文）。
  2. 在 README 增加归属说明，例如 “Based on satnaing/shadcn-admin (MIT), snapshot e16c87f”。
  3. 新增 `THIRD-PARTY-NOTICES.md`，至少列出上游 shadcn-admin、shadcn/ui、Radix UI、Tailwind CSS、Lucide 等直接依赖的许可证与版权行；正式发布前用 license 扫描工具逐项核对（本次未逐包验证所有依赖的 license 字段）。
  4. 保留上游 `public/images/*`、图标等资源的版权归属（它们随 MIT 仓库分发）；替换品牌 logo/名称时避免暗示官方关联。
  5. Clerk 的 logo、`ClerkFullLogo`、Clerk 品牌素材属于 Clerk 商标/品牌资产；按本 ticket 删除 Clerk 演示后，可一并移除这些素材，避免不必要的商标使用。
  6. README 中的 “Sponsored by Clerk” 徽章是赞助展示，不是 MIT 义务；删除不影响许可证合规。
- 上游 README 说 “This is not a starter project (template)”，但这只是作者表述，MIT 本身允许 fork、修改与再分发；不构成额外法律限制（仍建议保留归属）。

---

## 9. 风险、不确定性与后续建议

### 9.1 主要风险（按优先级）

1. **无 route guard**：未登录可直接访问所有保留页面；接入真实 JWT 时必须先补 `beforeLoad`。
2. **Mock auth 的安全假象**：可读 cookie 中的假 token、`exp` 不校验、任意凭据可登录；不能作为生产基础。
3. **没有任何 API 层**：users/settings/dashboard 全是静态数据或 toast，接入后端是主要工作量，不是配置工作。
4. **i18n 完全缺失**：文案散落在组件、sidebar 常量、Zod schema、toast 中，需要系统性抽取。
5. **删除 Clerk 后残留链接**：sign-in 的 Sign Up/Forgot password、sidebar 的 Auth/Errors 组、footer Terms/Privacy、profile 菜单的 Billing/New Team 等需要一并清理。
6. **头像资源缺失**：`/avatars/01.png` 等被引用但 `public/avatars/` 不存在；上游即如此，接入真实用户时需替换。
7. **shadcn 组件二次修改**：RTL 定制组件不能直接用 CLI 覆盖；升级时需手动合并（README 已警告）。

### 9.2 不确定性

- 审计基于快照 `e16c87f`（2026-06-11 UTC）。GitHub API 显示仓库在 2026-09-10 还有其他 push（可能来自分支/tag），但 `main` 的 HEAD 经 GitHub API 确认是 `e16c87f`；如果后续 main 前进，需要重新核对。
- 未运行 `pnpm install` / `bun install` / `vite build`，所有结论来自源码静态阅读；类型/构建层面的实际兼容性（尤其 bun + Vite 8 + Vitest 4）未验证。
- 后端 API 契约尚不存在；第 4.5、5.3、7.3 的字段/接口建议是通用方案，需要以后端设计为准。
- TanStack Router 的 search param 数字解析依赖其默认 JSON 反序列化行为；未实际启动应用验证 `?page=2` 在 `validateSearch` 中的类型表现（代码用 `.catch(1)` 兜底，风险较低）。
- `fullName` 列没有 accessor，排序行为在运行时可能无效；这是上游小瑕疵，未在浏览器中复现。
- 代码中没有发现逐文件的 copyright header；如果上游未来添加，fork 时需要保留。

### 9.3 建议的后续研究/实施顺序

1. 先做“裁剪 PR”：删除 demo/ Clerk/多余 auth/error 路由与 feature，裁剪 sidebar，清理链接与依赖，`pnpm build` 通过。
2. 再做“认证与 guard PR”：真实 login API、token 存储策略、`_authenticated` beforeLoad、401/refresh、登出。
3. 再做“API 层 PR”：axios instance、query keys、users 列表/CRUD、settings 读写。
4. 最后做“i18n PR”：LocaleProvider + zh-CN 资源 + sidebar/表单/错误页/日期格式 + 测试 locale 固定。
5. 最后补 `THIRD-PARTY-NOTICES` 与 README 归属说明。

---

## 10. 来源索引（raw GitHub URLs）

所有 URL 固定在审计快照 commit `e16c87f213a5ba5e45964e9b67c792105ec74d26`。

### 仓库与许可证

- 仓库主页: <https://github.com/satnaing/shadcn-admin>
- README: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/README.md>
- LICENSE: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/LICENSE>
- package.json: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/package.json>
- pnpm-lock.yaml: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/pnpm-lock.yaml>
- CI workflow: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/.github/workflows/ci.yml>
- Vite config: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/vite.config.ts>
- components.json: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/components.json>
- index.html: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/index.html>
- .env.example: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/.env.example>
- netlify.toml: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/netlify.toml>

### 路由与入口

- `src/main.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/main.tsx>
- `src/routes/__root.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/routes/__root.tsx>
- `src/routes/_authenticated/route.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/routes/_authenticated/route.tsx>
- `src/routes/_authenticated/index.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/routes/_authenticated/index.tsx>
- `src/routes/_authenticated/users/index.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/routes/_authenticated/users/index.tsx>
- `src/routes/_authenticated/settings/route.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/routes/_authenticated/settings/route.tsx>
- `src/routes/(auth)/sign-in.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/routes/(auth)/sign-in.tsx>
- `src/routes/clerk/route.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/routes/clerk/route.tsx>
- `src/routeTree.gen.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/routeTree.gen.ts>

### 认证与布局

- `src/stores/auth-store.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/stores/auth-store.ts>
- `src/lib/cookies.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/lib/cookies.ts>
- `src/features/auth/sign-in/components/user-auth-form.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/auth/sign-in/components/user-auth-form.tsx>
- `src/features/auth/sign-in/index.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/auth/sign-in/index.tsx>
- `src/features/auth/auth-layout.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/auth/auth-layout.tsx>
- `src/components/layout/authenticated-layout.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/components/layout/authenticated-layout.tsx>
- `src/components/sign-out-dialog.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/components/sign-out-dialog.tsx>
- `src/components/layout/nav-user.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/components/layout/nav-user.tsx>
- `src/components/profile-dropdown.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/components/profile-dropdown.tsx>
- `src/lib/handle-server-error.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/lib/handle-server-error.ts>

### 侧栏、i18n/RTL 与主题

- `src/components/layout/data/sidebar-data.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/components/layout/data/sidebar-data.ts>
- `src/components/command-menu.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/components/command-menu.tsx>
- `src/context/direction-provider.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/context/direction-provider.tsx>
- `src/context/theme-provider.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/context/theme-provider.tsx>
- `src/context/font-provider.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/context/font-provider.tsx>
- `src/components/config-drawer.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/components/config-drawer.tsx>
- `src/config/fonts.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/config/fonts.ts>

### Users

- `src/features/users/index.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/index.tsx>
- `src/features/users/data/users.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/data/users.ts>
- `src/features/users/data/schema.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/data/schema.ts>
- `src/features/users/data/data.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/data/data.ts>
- `src/features/users/components/users-columns.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/components/users-columns.tsx>
- `src/features/users/components/users-table.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/components/users-table.tsx>
- `src/features/users/components/users-action-dialog.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/components/users-action-dialog.tsx>
- `src/features/users/components/users-delete-dialog.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/components/users-delete-dialog.tsx>
- `src/features/users/components/users-invite-dialog.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/components/users-invite-dialog.tsx>
- `src/features/users/components/data-table-bulk-actions.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/users/components/data-table-bulk-actions.tsx>
- `src/hooks/use-table-url-state.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/hooks/use-table-url-state.ts>

### Settings

- `src/features/settings/index.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/settings/index.tsx>
- `src/features/settings/components/sidebar-nav.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/settings/components/sidebar-nav.tsx>
- `src/features/settings/components/content-section.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/settings/components/content-section.tsx>
- `src/features/settings/profile/profile-form.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/settings/profile/profile-form.tsx>
- `src/features/settings/account/account-form.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/settings/account/account-form.tsx>
- `src/features/settings/appearance/appearance-form.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/settings/appearance/appearance-form.tsx>
- `src/features/settings/notifications/notifications-form.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/settings/notifications/notifications-form.tsx>
- `src/features/settings/display/display-form.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/settings/display/display-form.tsx>
- `src/lib/show-submitted-data.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/lib/show-submitted-data.tsx>

### 错误页与演示页

- `src/features/errors/unauthorized-error.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/errors/unauthorized-error.tsx>
- `src/features/errors/not-found-error.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/errors/not-found-error.tsx>
- `src/features/errors/general-error.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/errors/general-error.tsx>
- `src/features/errors/maintenance-error.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/errors/maintenance-error.tsx>
- `src/routes/_authenticated/errors/$error.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/routes/_authenticated/errors/%24error.tsx>
- `src/features/dashboard/index.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/dashboard/index.tsx>
- `src/features/dashboard/components/overview.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/dashboard/components/overview.tsx>
- `src/features/dashboard/components/recent-sales.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/dashboard/components/recent-sales.tsx>
- `src/features/auth/sign-in/sign-in-2.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/auth/sign-in/sign-in-2.tsx>
- `src/features/auth/otp/components/otp-form.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/auth/otp/components/otp-form.tsx>
- `src/features/auth/forgot-password/components/forgot-password-form.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/auth/forgot-password/components/forgot-password-form.tsx>
- `src/features/auth/sign-up/components/sign-up-form.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/auth/sign-up/components/sign-up-form.tsx>

### 测试

- `src/stores/auth-store.test.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/stores/auth-store.test.ts>
- `src/features/auth/sign-in/components/user-auth-form.test.tsx`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/features/auth/sign-in/components/user-auth-form.test.tsx>
- `src/lib/cookies.test.ts`: <https://raw.githubusercontent.com/satnaing/shadcn-admin/e16c87f213a5ba5e45964e9b67c792105ec74d26/src/lib/cookies.test.ts>
