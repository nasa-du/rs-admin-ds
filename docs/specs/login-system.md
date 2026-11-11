# 生产可用登录系统实施规格

- Status: accepted
- Date: 2026-11-07
- Scope: 规格与交接文档；不包含生产代码实现
- Source of truth: `docs/adr/0001`–`0008`、地图「生产可用登录系统实施规格」下已关闭的决策票据、`CONTEXT.md`、`docs/research/*`

## 0. 文档定位与权威顺序

本文件是后续实现的总入口：把分散在 8 份 ADR、15 张决策票据与 3 份研究文档中的结论整合成一份可交接、可测试、无留白的中文实施规格。

发生冲突时，按以下顺序取权威：

1. **ADR 的最新 Amendments 与正文**（时间更晚的修订优先）；
2. **本规格**（用于整合、澄清与实现细节）；
3. **已关闭票据的历史评论**（仅提供背景；与本规格不一致时以本规格为准）。

实现过程中不得静默偏离本规格。新增不可逆决策（例如改变令牌传输方式、RBAC 模型、数据库大版本策略）必须新增或修订 ADR；纯实现细节可在 PR 中说明并由代码评审确认。所有“未决问题”已在 §15 给出决议或条件化方案，实施时不得留白。

本规格的术语以 `CONTEXT.md` 为准；代码标识符、表名、API 字段、错误码一律英文，叙述与界面文案中文优先。

---

## 1. 目标与非目标

### 1.1 目标

交付一套可指导实现的生产级登录与权限系统规格，满足：

- **后端**：Rust + Axum 0.8.9 + SQLx 0.9.0 + PostgreSQL 18 + JWT（EdDSA）。
- **前端**：fork `satnaing/shadcn-admin` 快照 `e16c87f`，裁剪后保留 Dashboard / Users / Roles / Audit Logs / Settings / Sign-in / Change-password / 错误页；zh-CN 默认、en-US 可选；包管理器 bun 1.4.2。
- **认证与授权**：前后端分离、同站子域部署下的安全令牌方案；内置三角色（超级管理员 / 管理员 / 普通用户）+ 自定义角色 + 权限勾选 + 用户多角色。
- **安全与运维**：Argon2id、PostgreSQL 三层登录限流与账号锁定（不引入 Redis）、审计日志后端 + 查询 UI、可观测性、多实例就绪的单机 Compose 部署。
- **交付物**：本规格 + 既有 ADR + `CONTEXT.md` 术语 + 可测试的验收标准 + 测试策略 + 分阶段里程碑。

### 1.2 成功定义

1. §14 的全部验收标准可在 CI 或发布验收环境中执行并通过。
2. 后续实现团队无需再向决策者询问“用哪种令牌 / 谁能改谁 / 数据库怎么升级 / 前端保留哪些页面”等已决问题。
3. 所有不可逆选择都能追溯到 ADR；所有 UI 细节都能追溯到本规格或对应票据。

### 1.3 非目标（明确不做）

- 公开注册、忘记密码、邮箱验证、MFA、社交登录、企业 SSO/LDAP、任何邮件服务或邮件通知。
- 多租户、API Key、OAuth/OIDC Provider、模拟登录。
- Redis、进程内共享状态、跨标签共享 access token。
- 用户硬删除 / 软删除、批量删除、批量启停、批量邀请、管理员永久密码、查看既有密码或已签发临时密码。
- 终端用户的会话 / 设备列表 UI（管理员撤销用户全部会话的后端能力保留）。
- 审计日志导出 / 批量下载、审计链校验 UI、外部 WORM 归档、日志 SaaS。
- Dashboard 的真实业务数据（保留受保护占位壳）。
- 上游页面 tasks、apps、chats、help-center、sign-in-2、Clerk、sign-up、forgot-password、otp、错误演示容器。
- 本次 effort 的生产代码实现：本文件只交付规格；实现按 §13 里程碑另行排期。
- Kubernetes、永久 staging、蓝绿 / 双实例、多区域多活、服务网格、GitOps。

---

## 2. 架构与仓库布局

### 2.1 部署架构

```text
Browser
  │ HTTPS
  ▼
nginx gateway  ── admin.example.com ──► web (nginx static, SPA)
  │             └─ api.example.com   ──► api (Axum, :8080)
  │                                         │
  │                                         ├─ PostgreSQL 18.6（同机 db 或外部 DATABASE_URL）
  │                                         └─ metrics :9100 ◄─ Prometheus（Compose 内网）
  │
  ├─ Let's Encrypt / certbot
  └─ systemd timer ──► maintenance / backup 一次性容器
                         └─ pgBackRest ──► S3 兼容对象存储（WAL + 每日全量）

observability Compose：Prometheus、Alertmanager、Grafana、Loki、Promtail、
postgres_exporter、node_exporter、webhook 适配器
```

- 默认拓扑：**同站不同源**。SPA `https://admin.example.com`，API `https://api.example.com`；两者共享 registrable domain，cross-origin 但 same-site，refresh cookie 属于第一方 cookie。
- 实际域名、证书、DNS、网关归属未定，按 §10.2 的上线检查清单收集。若无法同站，按 ADR-0001 回退到同源 BFF 或同源反代；**不把 `SameSite=None` 第三方 Cookie 作为生产主方案**。
- 多实例是设计前提：会话、刷新轮换、限流、锁定、审计链全部落 PostgreSQL；首发 1 个 API 副本，扩容 checklist 见 §10.11。
- 不引入 Redis；不把任何共享状态只放进程内存。

### 2.2 仓库布局（实现阶段采用）

```text
rs-admin-ds/
├── Cargo.toml                    # workspace：members = ["backend/crates/*"]
├── rust-toolchain.toml           # channel 固定到当前 stable；rust-version = 1.94
├── Cargo.lock                    # 提交
├── .env.example
├── backend/
│   ├── crates/
│   │   ├── api/                  # Axum 路由、中间件、DTO、OpenAPI、错误映射
│   │   ├── domain/               # 实体、权限目录、授予子集/管理范围/超管不变量（纯逻辑）
│   │   ├── store/                # SQLx 查询、事务、迁移执行、仓储实现
│   │   ├── security/             # Argon2、JWT、刷新派生、限流、可信代理、密码策略
│   │   ├── audit/                # 审计事件定义、链哈希、写入器
│   │   ├── config/               # 配置加载、secret 文件、启动校验
│   │   └── cli/                  # bootstrap-super-admin、recover-super-admin、维护命令
│   ├── migrations/               # UTC 时间戳前缀的 .sql，不可修改
│   ├── .sqlx/                    # cargo sqlx prepare 生成并提交
│   └── openapi.json              # utoipa 生成并提交
├── frontend/                     # shadcn-admin 裁剪 fork
│   ├── src/routes/
│   ├── src/features/
│   ├── src/api/generated/        # orval 生成并提交
│   ├── src/i18n/
│   ├── src/lib/format.ts
│   ├── bun.lock
│   └── package.json              # packageManager: bun@1.4.2
├── deploy/
│   ├── compose.yaml
│   ├── compose.observability.yaml
│   ├── nginx/
│   ├── pgbackrest/
│   ├── systemd/
│   └── secrets/README.md         # 生产 secret 放 /etc/rs-admin/secrets，不进仓库
└── docs/
    ├── adr/
    ├── research/
    ├── specs/login-system.md
    └── agents/
```

模块边界：

- `api` 只做 HTTP 适配：认证中间件、参数校验、事务边界、DTO ↔ domain、ProblemDetails。不得在 handler 里重复实现授权规则。
- `domain` 不依赖 SQLx、Axum、HTTP；权限解析、授予子集、管理范围、自我操作、最后超管不变量、状态机以纯函数/策略对象表达，便于单元测试。
- `store` 是唯一写 SQL 的层；所有多语句业务动作在显式事务中完成；审计写入与业务动作同事务（见 §8.5）。
- `security` 封装 Argon2、JWT 签发/验签、刷新派生、限流计数、可信代理解析；不直接构造 HTTP 响应。
- `audit` 提供统一 `AuditEvent` 与 `AuditWriter`；所有事件必须经过目录校验，禁止自由字符串。
- `cli` 复用 `domain`/`store`/`security`，但**不暴露为 HTTP 端点**。

### 2.3 依赖方向

```text
api ──► domain ◄── store
 │        ▲          │
 ├──► security ──────┤
 ├──► audit ─────────┤
 └──► config         │
cli ──► domain/store/security/audit/config
```

禁止反向依赖：`domain` 不引用 `api`/`store`/`security`；`store` 不引用 `api`；`audit` 不引用 `api`。

---

## 3. 技术栈与固定版本

版本来自 `docs/research/axum-stack.md` 与 `docs/research/shadcn-admin-audit.md`（2026-11-05/07 核验）。实现首日应再跑一次 `cargo search` / `cargo update` / `bun install` 验证，但大版本不随意漂移。

| 层 | 依赖 / 工具 | 版本 | 备注 |
|---|---|---|---|
| Rust | toolchain | `rust-version = 1.94` | SQLx 0.9 的 MSRV 是 1.94；CI 用当前 stable ≥1.94 |
| HTTP | `axum` | `0.8.9` | 路径参数 `/{id}`；`axum::serve` |
| 中间件 | `tower` / `tower-http` | `0.5.3` / `0.7.1` | 显式启用 `trace`、`request-id`、`timeout`、`limit` |
| 数据库 | `sqlx` | `0.9.0` | features：`postgres`、`runtime-tokio`、`tls-rustls-ring`、`macros`、`migrate`、`json`、`uuid`、`time`/`chrono` 按实现选择 |
| SQLx CLI | `sqlx-cli` | `0.9.0` | 与 crate minor 对齐；`--no-default-features --features rustls,postgres` |
| 密码 | `argon2` | `0.6.0` | Argon2id v19，PHC 字符串；启用 `parallel` |
| JWT | `jsonwebtoken` | `11.1.0` | `rust_crypto` backend；EdDSA |
| OpenAPI | `utoipa` / `utoipa-axum` | `6.0.0` / `0.3.0` | OpenAPI 3.1 |
| Swagger UI | `utoipa-swagger-ui` | `10.0.2` | 仅开发/内网；生产关闭 |
| 测试容器 | `testcontainers` / `testcontainers-modules` | `0.27.3` / `0.15.0` | modules 0.15 只兼容 core 0.27.x |
| UUID | `uuid` | `1.29.0` | 应用侧生成审计/请求 id |
| 指标 | `metrics` / `metrics-exporter-prometheus` | `0.24.6` / `0.18.3` | 只用 recorder 时关闭默认 feature |
| 数据库镜像 | `postgres` | `18.6-alpine` | 固定 tag；不用浮动的 `18-alpine` |
| 网关 | `nginx` | `1.29.8-alpine` | `limit_req`、TLS、静态托管 |
| 前端运行时（构建） | `oven/bun` | `1.4.2` | 与本地/CI/Docker 固定一致 |
| 前端框架 | React / Vite | `^19.2.5` / `^8.0.8` | 沿用上游 |
| 路由 | `@tanstack/react-router` | `^1.168.22` | file-based |
| 服务端状态 | `@tanstack/react-query` | `^5.99.0` | |
| 表格 / 表单 | `@tanstack/react-table` / `react-hook-form` + `zod` | `^8.21.3` / `^7.72.1` + `^4.3.6` | |
| HTTP 客户端 | `axios` | `^1.15.0` | orval mutator |
| 客户端生成 | `orval` | `8.46.0` | Node ≥22.18；Bun 兼容性实现首日验证 |
| 回退生成 | `openapi-typescript` + `openapi-fetch` | `7.13.0` + 对应版本 | 仅 orval/Bun 不兼容时启用 |
| i18n | `i18next` + `react-i18next` | 实现首日锁定稳定版 | 静态双语资源 |
| 测试 | `vitest` + `@vitest/browser-playwright` | `^4.1.4` | browser mode + MSW |
| 认证（上游，删除） | `@clerk/react` | `^6.4.3` | 从 fork 删除 |
| 清理依赖 | `@faker-js/faker`、`recharts`、`input-otp` | 上游版本 | 删除 |

版本偏差处理：

- **orval 8.46 在 Bun 1.4.2 下不兼容** → 按 ADR-0004/0006 回退 `openapi-typescript` + `openapi-fetch`；认证、错误、刷新契约不变，不退化为手写业务客户端。
- **SQLx 0.9 无法使用 Rust 1.94** → 降级到 SQLx 0.8.6 是备选，但会失去 `sqlx.toml` 等能力；需记录 ADR 修订。
- **生产 Argon2 基准超过 300 ms** → 按 §8.1 顺序回退参数。

---

## 4. 数据模型

### 4.1 通用约定

- 主键一律 `uuid PRIMARY KEY DEFAULT uuidv7()`（PostgreSQL 18 内置；不做 PG17 回退）。
- 时间一律 `timestamptz`，由应用或数据库 `now()` 写入；API 输出 RFC3339 UTC（毫秒精度，`...Z`）。
- `created_at` / `updated_at` 由应用显式维护，不用触发器。
- 外键一律 `ON DELETE RESTRICT`。用户永不删除；自定义角色软删除；会话用 `revoked_at`。
- 标识符（username/email/角色名）在应用层规范化后存储；数据库用 `CHECK` + 唯一索引兜底。
- 所有 JSON 字段使用 `jsonb`；审计 `details` 为开放 object。
- 迁移文件使用 UTC 时间戳前缀，提交仓库且不可修改；生产不在应用启动时自动迁移（§10.9）。

### 4.2 表结构

以下 DDL 是规格级定义；实现可调整类型细节（如 `uuid` vs `text` 的 request_id），但列语义、约束与索引必须保留。

```sql
CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    username text NOT NULL,
    email text NOT NULL,
    display_name text NOT NULL,
    phone text,
    password_hash text NOT NULL,
    status text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    must_change_password boolean NOT NULL DEFAULT false,
    temporary_password_expires_at timestamptz,
    token_version bigint NOT NULL DEFAULT 0,
    failed_login_count integer NOT NULL DEFAULT 0,
    locked_until timestamptz,
    last_failed_login_at timestamptz,
    last_login_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT users_username_lower CHECK (username = lower(username)),
    CONSTRAINT users_email_lower CHECK (email = lower(email)),
    CONSTRAINT users_username_format CHECK (username ~ '^[a-z0-9][a-z0-9._-]{2,31}$'),
    CONSTRAINT users_display_name_len CHECK (char_length(display_name) BETWEEN 1 AND 64),
    CONSTRAINT users_phone_len CHECK (phone IS NULL OR char_length(phone) <= 32)
);
CREATE UNIQUE INDEX users_username_uq ON users (username);
CREATE UNIQUE INDEX users_email_uq ON users (email);
CREATE INDEX users_username_trgm ON users USING gin (username gin_trgm_ops);
CREATE INDEX users_email_trgm ON users USING gin (email gin_trgm_ops);
CREATE INDEX users_display_name_trgm ON users USING gin (display_name gin_trgm_ops);

CREATE TABLE roles (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    name text NOT NULL,
    description text,
    is_builtin boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz,
    CONSTRAINT roles_name_len CHECK (char_length(name) BETWEEN 1 AND 64),
    CONSTRAINT roles_description_len CHECK (description IS NULL OR char_length(description) <= 500),
    CONSTRAINT roles_builtin_not_deleted CHECK (NOT is_builtin OR deleted_at IS NULL)
);
CREATE UNIQUE INDEX roles_name_active_uq ON roles (lower(name)) WHERE deleted_at IS NULL;

CREATE TABLE permissions (
    key text PRIMARY KEY,
    group_key text NOT NULL,
    display_name_zh text NOT NULL,
    display_name_en text NOT NULL
);

CREATE TABLE role_permissions (
    role_id uuid NOT NULL REFERENCES roles(id) ON DELETE RESTRICT,
    permission_key text NOT NULL REFERENCES permissions(key) ON DELETE RESTRICT,
    PRIMARY KEY (role_id, permission_key)
);

CREATE TABLE user_roles (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    role_id uuid NOT NULL REFERENCES roles(id) ON DELETE RESTRICT,
    assigned_at timestamptz NOT NULL DEFAULT now(),
    assigned_by uuid REFERENCES users(id) ON DELETE RESTRICT,
    PRIMARY KEY (user_id, role_id)
);
CREATE INDEX user_roles_role_idx ON user_roles (role_id);

CREATE TABLE auth_sessions (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    idle_expires_at timestamptz NOT NULL,
    absolute_expires_at timestamptz NOT NULL,
    authenticated_at timestamptz NOT NULL,
    revoked_at timestamptz,
    revoke_reason text,
    ip inet,
    user_agent text
);
CREATE INDEX auth_sessions_user_active_idx
    ON auth_sessions (user_id, last_seen_at)
    WHERE revoked_at IS NULL;

CREATE TABLE auth_refresh_tokens (
    token_hash bytea PRIMARY KEY,
    session_id uuid NOT NULL REFERENCES auth_sessions(id) ON DELETE RESTRICT,
    rotation_counter bigint NOT NULL,
    issued_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    used_at timestamptz,
    replaced_by_hash bytea UNIQUE,
    revoked_at timestamptz,
    revoked_reason text,
    CONSTRAINT auth_refresh_tokens_counter_nonneg CHECK (rotation_counter >= 0)
);
CREATE INDEX auth_refresh_tokens_session_idx
    ON auth_refresh_tokens (session_id, rotation_counter);

CREATE TABLE auth_login_failures (
    key_hash bytea PRIMARY KEY,
    failed_count integer NOT NULL DEFAULT 0,
    locked_until timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX auth_login_failures_updated_idx ON auth_login_failures (updated_at);

CREATE TABLE auth_login_throttle (
    scope text NOT NULL CHECK (scope IN ('ip', 'global')),
    key_hash bytea NOT NULL,
    window_start timestamptz NOT NULL,
    failures integer NOT NULL DEFAULT 0,
    locked_until timestamptz,
    last_failure_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (scope, key_hash)
);
CREATE INDEX auth_login_throttle_last_failure_idx ON auth_login_throttle (last_failure_at);

CREATE TABLE audit_logs (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    occurred_at timestamptz NOT NULL DEFAULT now(),
    actor_user_id uuid,
    actor_label text NOT NULL,
    action text NOT NULL,
    target_type text NOT NULL,
    target_id uuid,
    target_label text,
    result text NOT NULL CHECK (result IN ('success', 'failure', 'denied')),
    severity text NOT NULL CHECK (severity IN ('info', 'warning', 'error', 'critical')),
    ip inet,
    user_agent text,
    request_id uuid NOT NULL,
    details jsonb NOT NULL DEFAULT '{}'::jsonb,
    chain_seq bigint NOT NULL UNIQUE,
    prev_hash bytea NOT NULL,
    row_hash bytea NOT NULL,
    hmac_key_id text NOT NULL
);
CREATE INDEX audit_logs_occurred_at_idx ON audit_logs (occurred_at DESC);
CREATE INDEX audit_logs_actor_idx ON audit_logs (actor_user_id, occurred_at DESC);
CREATE INDEX audit_logs_action_idx ON audit_logs (action, occurred_at DESC);
CREATE INDEX audit_logs_target_idx ON audit_logs (target_type, target_id, occurred_at DESC);
CREATE INDEX audit_logs_severity_idx ON audit_logs (severity, occurred_at DESC);
CREATE INDEX audit_logs_ip_idx ON audit_logs (ip, occurred_at DESC);
CREATE INDEX audit_logs_request_id_idx ON audit_logs (request_id);

CREATE TABLE audit_chain_head (
    id smallint PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    last_seq bigint NOT NULL,
    last_hash bytea NOT NULL,
    updated_at timestamptz NOT NULL
);
```

`audit_chain_head` 迁移时插入 `(1, 0, 32 个零字节, now())`。

### 4.3 数据语义

- `users.username`：3–32 字符，ASCII `[a-z0-9][a-z0-9._-]*`，不含 `@`、空白或 Unicode；存储前小写。保留 `admin`、`root`、`system` 等少量名称（具体列表由 domain 常量维护）。
- `users.email`：`trim` + Unicode NFC + 小写后存储；≤254；格式校验在 API。
- `users.display_name`：1–64 Unicode；缺省为 username；不要求唯一。
- `users.phone`：可选，≤32，`trim`，不参与登录、不要求唯一，仅管理员维护。
- `users.password_hash`：完整 Argon2id PHC 字符串；不拆分 salt/params/version 列。
- `users.status`：只有 `active` / `disabled`；停用是可逆管理动作。
- `users.must_change_password` + `temporary_password_expires_at`：共同派生“待首次登录 / 临时密码过期”；不新增状态。
- `users.token_version`：递增后该用户此前签发的全部 access token 立即失效。
- `users.failed_login_count` / `locked_until` / `last_failed_login_at`：账号层限流状态；24 小时无失败自动衰减。
- `roles.name`：内置角色固定为 `user`、`admin`、`super_admin`；自定义角色名称在活跃角色中不区分大小写唯一，软删除后名称可复用。
- `roles.deleted_at`：仅自定义角色可软删除；软删除后不出现在列表与分配候选。
- `permissions`：代码目录的镜像，只在迁移/启动时同步；不接受运行时创建。
- `user_roles`：基线 `user` 对每个用户显式落一行；`assigned_by` 可空（CLI 引导时为 null）。
- `auth_sessions.id`：同时是 access token 的 `sid`；`authenticated_at` 是 step-up 的权威时间。
- `auth_refresh_tokens`：主键是原始令牌的 SHA-256；`rotation_counter` 从 0 开始；`replaced_by_hash` 指向同会话的下一枚令牌哈希。
- `auth_login_failures`：只用于**未知标识**的账号层计数；已知用户使用 `users` 上的列。
- `auth_login_throttle`：IP 层与全局层共用；`key_hash = HMAC-SHA256(THROTTLE_KEY, 规范化键)`；不保存明文 IP。
- `audit_logs`：不建外键；运行角色只 `INSERT`/`SELECT`；`target_label` 是事件发生时的名称快照。
- `audit_chain_head`：单行链表头；所有审计写入先 `SELECT ... FOR UPDATE` 该行。

### 4.4 迁移、目录同步与种子

- 迁移：`sqlx::migrate!()` 内嵌；文件 UTC 时间戳前缀；只前进、不自动回滚；不可修改（SQLx checksum）。CI 在空库与上一发布 schema 上各跑一次。
- `.sqlx` offline 元数据提交；CI 跑 `cargo sqlx prepare --check --workspace -- --all-targets --all-features`；构建用 `SQLX_OFFLINE=true`。
- `.gitattributes` 固定 `*.sql text eol=lf`。
- 权限目录与内置角色权限集在**启动时**从代码同步：事务内取 advisory lock，upsert `permissions` 与 `role_permissions`，创建 `user`/`admin`/`super_admin` 三行内置角色；v1 不删除目录中不存在的旧项。
- 生产不 seed 任何用户；首个超管只走 CLI 引导；开发/测试 fixture 与生产种子严格分离。
- PG18 作为硬下限：使用 `uuidv7()` 与 `pg_trgm`；生产用 SCRAM + `sslmode=verify-full`；不使用 PG18 `oauth` HBA。

### 4.5 数据生命周期与清理

- 过期/撤销的 `auth_refresh_tokens`：保留 30 天后由维护任务删除（先删 token，再删 session，满足 `RESTRICT`）。
- 已撤销或已过期的 `auth_sessions`：保留 30 天后删除；删除前确认其 refresh token 已清理。
- `auth_login_failures` / `auth_login_throttle`：超过 30 天且未锁定的行删除。
- `audit_logs`：按 severity 分层保留：`info`/`warning` 180 天，`error`/`critical` 365 天；可配置且不得低于下限。清理任务写入 `system_audit_logs_purged` 并更新链头。
- 维护任务用固定 key 的 `pg_try_advisory_lock` 防止并发执行；未获得锁则退出 0。

---

## 5. 认证与会话

### 5.1 Access token

- 格式：JWT，header `typ=at+jwt`，签名算法 **EdDSA（Ed25519）**，header 带 `kid`；使用 `jsonwebtoken 11.1.0` + `rust_crypto`。
- TTL：**15 分钟**；校验 `exp - iat <= 15 min` 作为最大寿命；clock skew leeway **60 秒**。
- Claims：

| Claim | 值 |
|---|---|
| `sub` | `users.id`（UUIDv7） |
| `sid` | `auth_sessions.id`（UUIDv7） |
| `ver` | `users.token_version` |
| `iss` | `https://api.example.com`（配置项 `JWT_ISSUER`） |
| `aud` | `rs-admin-ds-api`（配置项 `JWT_AUDIENCE`） |
| `exp` / `iat` / `nbf` | NumericDate（秒） |
| `jti` | UUIDv7；用于审计/追踪，不做 denylist |
| `typ` | `at+jwt` |
| `auth_time` | Unix 秒；最近一次密码认证时间；refresh 时从 `auth_sessions.authenticated_at` 复制，不重置 |

- **不包含** roles、permissions、email、username。角色与权限每次请求从 PostgreSQL 解析；前端渲染所需权限由 `/auth/me` 返回，服务端不信任前端。
- 存储：仅 JavaScript 内存（Zustand 模块变量）；不写 `localStorage` / `sessionStorage` / JS 可读 Cookie。
- 验证顺序：固定算法 allowlist → 按本地可信 key set 的 `kid` 取 key → 验签 → 校验 `typ`/`iss`/`aud`/`exp`/`nbf` → 查会话与用户 → 校验 `token_version`。拒绝 `none`；拒绝从 token 中的 `jku`/`jwk`/`x5u` 取 key；未知 `kid` 直接拒绝。
- 失败统一 401 + `WWW-Authenticate: Bearer realm="rs-admin"`。

### 5.2 Refresh token

- 格式：初始令牌为 256-bit CSPRNG 不透明随机串（base64url）；数据库只存 `SHA-256(raw)`，不存明文。
- Cookie：

```http
Set-Cookie: __Host-refresh=<raw>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=43200
```

- 不设 `Domain`；host-only 到 `api.example.com`。
- 服务端只在 `/auth/login`、`/auth/refresh`、`/auth/logout`、`/auth/logout-all`、`/auth/change-password` 读取该 Cookie；其他端点忽略 `Cookie`。`/auth/change-password` 仅用 Bearer 鉴权，读取 Cookie 只为轮换当前会话的 refresh token。
- 每次刷新轮换；`Max-Age = min(43200, absolute_expires_at - now())`；服务端仍强制 idle/absolute，Cookie 过期时间不是安全边界。
- **派生式轮换**：`new_raw = base64url(HMAC-SHA256(REFRESH_DERIVATION_KEY, old_raw || ":" || new_counter))`，其中 `new_counter = old.rotation_counter + 1`。数据库仍只存每枚令牌的 SHA-256；`replaced_by_hash` 记录后继；派生密钥独立于 JWT 签名密钥。
- **30 秒 grace**：旧令牌在其 `used_at` 后 30 秒内再次出现，沿 `replaced_by_hash` 链现算后继，返回当前有效令牌；不新增轮换、不写审计。链上任一步缺失、已撤销或超出 grace → 按刷新重放处理。
- **刷新重放**：已轮换令牌在 grace 外再次出现 → 撤销该会话的整个令牌族并撤销会话（`revoke_reason=token_reuse`），写 `authn_refresh_reuse_detected`（critical），清 Cookie，返回 401 `refresh_reuse_detected`。不递增 `token_version`（仅影响该会话）。
- 客户端必须单飞刷新（§9.4）。
- 正常轮换：校验会话未撤销、idle/absolute 未过期、用户启用且 `token_version` 一致；标记旧 token `used_at`；插入新 token 行；更新 `replaced_by_hash`；按 60 秒节流更新 `last_seen_at`/`idle_expires_at`；签发新 access token；Set-Cookie；返回 `{access_token, token_type, expires_in}`。

### 5.3 会话生命周期

- 登录创建会话：`idle_expires_at = now() + 30 min`，`absolute_expires_at = now() + 12 h`，`authenticated_at = now()`。
- 每用户最多 **5 个活跃会话**。第 6 次登录在同一事务内锁用户行（`SELECT ... FROM users WHERE id = $1 FOR UPDATE`），按 `last_seen_at` 撤销最旧会话（`revoke_reason=excess_sessions`），写 `authn_session_evicted`，再创建新会话。
- `last_seen_at` / `idle_expires_at` 最多每 60 秒更新一次；idle 超时检查每请求执行；absolute 时间固定不延长。
- 会话状态机：

```text
active ── logout ──────────────► revoked(logout)
       ├─ logout-all / reset ──► revoked(logout_all | admin_revoke)
       ├─ change-password ─────► revoked(password_change)
       ├─ disable ──────────────► revoked(account_disabled)
       ├─ refresh reuse ────────► revoked(token_reuse)
       ├─ 5-session cap ────────► revoked(excess_sessions)
       ├─ idle timeout ─────────► expired
       └─ absolute timeout ─────► expired
```

- 撤销原因取值：`logout`、`logout_all`、`password_change`、`account_disabled`、`token_reuse`、`admin_revoke`、`excess_sessions`。`revoke_reason` 由应用校验，不加数据库 CHECK，便于未来扩展。

### 5.4 每请求认证管线

对每个受保护请求，按以下顺序执行：

1. 生成/透传 `request_id`（服务端 UUIDv7，忽略客户端传入）；写回 `X-Request-Id`。
2. 解析 `Authorization: Bearer`；JWT 验签与 claims 校验。
3. 从 PostgreSQL 加载用户与会话（可一次 join）：用户不存在 → 401 `session_invalid`；用户 `disabled` → **403 `account_disabled`**；会话不存在/已撤销/idle 或 absolute 过期 → 401 `session_invalid`；`users.token_version != jwt.ver` → 401 `session_invalid`。
4. 解析有效角色与权限（超级管理员直接放行）；受限会话检查端点 allowlist。
5. 端点级权限检查；失败 403 `forbidden`。
6. 管理范围 / 目标资格检查；失败 403 `out_of_scope` / `permission_subset_violation` / `role_immutable`。
7. step-up 检查（仅敏感端点）；`now - authenticated_at > 300s` → 403 `reauth_required`，`details.max_age = 300`。
8. 执行业务动作；审计写入与业务动作同事务（无业务事务的事件在返回前写入）。
9. 数据库不可用 → 503 `service_unavailable`（fail-closed）；不得只看 JWT 放行。

顺序说明：先权限/范围、再 step-up、最后变更（ADR-0005 §5.8）；这样可以避免对无权限请求弹重新认证。

受限会话（`must_change_password=true`）只允许：`GET /auth/me`、`POST /auth/refresh`、`POST /auth/change-password`、`POST /auth/logout`。其他端点（含 `PATCH /auth/me`、`POST /auth/logout-all`、`POST /auth/reauthenticate`）返回 403 `password_change_required`。

### 5.5 登录时序

```mermaid
sequenceDiagram
    participant B as Browser
    participant G as nginx
    participant A as API
    participant P as PostgreSQL
    participant U as Audit

    B->>G: POST /api/v1/auth/login {identifier,password}
    G->>G: limit_req /auth/* 60r/m
    G->>A: 转发 + Origin/X-Requested-With
    A->>A: 解析标识、规范化、生成 request_id
    A->>P: 查询账号层/IP/全局锁定状态
    alt 已锁定
        A->>A: dummy Argon2id 验证
        A->>U: authn_login_lock / authn_login_rate_limited
        A-->>B: 429 + Retry-After
    else 未锁定
        A->>P: 按 identifier 查 user（未知则 dummy hash）
        A->>A: Argon2id 验证
        alt 失败
            A->>P: 原子更新失败计数/锁定
            A->>U: authn_login_fail（+ 阈值时 authn_login_lock）
            A-->>B: 401 invalid_credentials / 429 login_locked
        else 成功但停用
            A->>U: authn_login_fail(account_disabled)
            A-->>B: 403 account_disabled
        else 成功但临时密码过期
            A->>U: authn_login_fail(temporary_password_expired)
            A-->>B: 403 temporary_password_expired
        else 成功
            A->>P: BEGIN; 锁用户行; 会话上限淘汰; 创建 session
            A->>P: 创建 refresh token; 写 authn_login_success / authn_session_evicted; COMMIT
            A-->>B: 200 {access_token, user} + Set-Cookie
        end
    end
```

登录细节：

- 登录输入先 `trim`；含 `@` 按 email 查找，否则按 username 查找；两者规范化后匹配。
- 未知标识与错误密码都执行 Argon2id/dummy 验证，返回统一 401 `invalid_credentials`；不得通过状态码、文案或响应时间暴露用户是否存在。
- 锁定期间仍执行一次 dummy 验证；返回 429 + `Retry-After`；即使密码正确也不放行。
- 成功登录重置账号层失败计数，保留 IP 层计数；更新 `last_login_at`。
- 登录响应 `Cache-Control: no-store`；refresh token 不出现在 JSON、URL 或日志。

### 5.6 刷新时序

```mermaid
sequenceDiagram
    participant B as Browser
    participant A as API
    participant P as PostgreSQL
    participant U as Audit

    B->>A: POST /auth/refresh（Cookie + CSRF 头）
    A->>A: 校验 Origin/X-Requested-With/JSON
    A->>P: SELECT session/token FOR UPDATE
    alt 会话/用户无效
        A-->>B: 401 session_invalid / 403 account_disabled
    else token 未使用
        A->>P: used_at=now; 派生新 token; 插入; replaced_by_hash
        A->>P: 节流更新 last_seen/idle
        A->>U: 无单独审计（成功刷新不刷事件）
        A-->>B: 200 新 access_token + 轮换 Set-Cookie
    else token 已使用且 used_at ≤ 30s
        A->>A: 沿链现算当前有效后继
        A-->>B: 200 同一后继 + Set-Cookie（幂等）
    else token 已使用且 used_at > 30s
        A->>P: 撤销整个令牌族 + 会话
        A->>U: authn_refresh_reuse_detected (critical)
        A-->>B: 401 refresh_reuse_detected + 清 Cookie
    end
```

### 5.7 登出、全部登出与改密

- **Logout**：`POST /auth/logout`，读 refresh Cookie；撤销当前会话（`logout`），清 Cookie，204；幂等（无有效会话时也返回 204）。审计 `authn_logout` 仅在确实撤销会话时写入。
- **Logout all**：`POST /auth/logout-all`，Bearer；撤销该用户全部会话，`token_version++`，清 Cookie，204；审计 `authn_logout_all`。受限会话返回 403 `password_change_required`。
- **自助改密**：`POST /auth/change-password`，Bearer，`{current_password,new_password}`：
  1. 校验当前密码；错误 422 `invalid_current_password`，并计入账号/IP 失败计数（防止持有会话后无限尝试当前密码），写 `authn_password_change_fail`。
  2. 新密码过策略；不得等于当前密码（422 `password_unchanged`）；不得命中 blocklist / 等于标识（422 `password_policy_violation`）。
  3. 成功后清除 `must_change_password` 与 `temporary_password_expires_at`；撤销其他全部会话；`token_version++`；保留当前会话，更新 `authenticated_at=now()`，轮换当前会话的 refresh token，重签 access token（携带新 `ver` 与 `auth_time`）；Set-Cookie；审计 `authn_password_changed`。
  4. 响应 `{access_token, token_type, expires_in}`；当前设备不掉线，其他设备失效。
- **管理员重置凭据**：撤销目标用户全部会话 + `token_version++` + 清除账号层锁定；签发临时密码；审计 `user_password_reset`（`method=generated|manual`）。

### 5.8 重新认证（Step-up）

- 端点：`POST /auth/reauthenticate`，Bearer，`{password}`；不读 refresh Cookie，不要求 CSRF 头。
- 成功：更新 `auth_sessions.authenticated_at = now()`；重置账号层失败计数（保留 IP 层）；签发新 access token（同一 `sid`/`ver`/`token_version`，`auth_time=now()`）；**不轮换 refresh token、不建新会话**；返回 `{access_token, token_type, expires_in, auth_time}`。
- 失败：422 `invalid_current_password`；计入账号/IP 失败层；写 `authn_reauth_fail`。
- 敏感操作（`now - authenticated_at > 300s`）返回 403 `reauth_required`，`details.max_age=300`；写 `authz_step_up_denied`。不用 401，避免前端把它当作令牌过期而触发刷新循环。
- 受限会话返回 403 `password_change_required`。

敏感操作清单：

| 端点 | 是否要求 step-up |
|---|---|
| `POST /users` | 是（一律） |
| `PATCH /users/{id}` | 仅当请求包含 `username` 或 `email` |
| `PUT /users/{id}/status` | 是 |
| `POST /users/{id}/reset-password` | 是 |
| `PUT /users/{id}/roles` | 是 |
| `DELETE /users/{id}/sessions` | 是 |
| `POST /roles`、`PATCH /roles/{id}`、`PUT /roles/{id}/permissions`、`DELETE /roles/{id}` | 是 |
| `POST /users/{id}/unlock` | 否 |
| `POST /auth/logout`、`POST /auth/logout-all`、`PATCH /auth/me`（display_name） | 否 |
| `GET /audit-logs`、`GET /audit-logs/{id}` | 否 |

### 5.9 撤销矩阵

| 触发 | 影响的会话 | `token_version` | refresh Cookie | 审计 |
|---|---|---|---|---|
| 用户登出 | 当前会话 | 不变 | 清除当前 | `authn_logout` |
| 全部登出 | 全部会话 | +1 | 清除当前 | `authn_logout_all` |
| 自助改密 | 其他会话；当前会话保留并轮换 | +1 | 当前轮换 | `authn_password_changed` |
| 管理员重置凭据 | 目标用户全部会话 | +1 | 目标浏览器下次失败 | `user_password_reset` |
| 停用用户 | 目标用户全部会话 | +1 | 目标浏览器下次失败 | `user_status_changed` |
| 管理员撤销会话 | 目标用户全部会话 | +1 | 目标浏览器下次失败 | `user_sessions_revoked` |
| 刷新重放 | 该会话令牌族 | 不变 | 清除当前 | `authn_refresh_reuse_detected` |
| 第 6 个会话淘汰 | 最旧会话 | 不变 | 该设备下次刷新失败 | `authn_session_evicted` |
| 角色/权限变更 | 不变 | 不变 | 不变 | 角色/用户角色事件 |

停用与重置都会撤销会话并递增版本；但每请求管线在 JWT 验签后先加载用户状态，因此停用用户的下一受保护请求稳定返回 **403 `account_disabled`**（即使会话已撤销）；若 access token 已过期，则先返回 401 `access_token_invalid`，随后的 refresh 返回 403 `account_disabled`。重置导致的会话撤销则表现为 401 `session_invalid`。前端对这些路径都必须安全登出（§9.3）。

---

## 6. RBAC 权限模型

### 6.1 权限目录

权限由后端代码定义，key 为 `resource.action`（资源复数、action snake_case），带中英文显示名与分组；数据库 `permissions` 表只是镜像，只在迁移/启动时同步，**不接受运行时创建**。

| 分组 | 权限 key | 中文显示名 | English |
|---|---|---|---|
| users | `users.read` | 查看用户 | View users |
| users | `users.create` | 创建用户 | Create users |
| users | `users.update` | 编辑用户 | Edit users |
| users | `users.set_status` | 启用/停用用户 | Enable/disable users |
| users | `users.reset_password` | 重置用户密码 | Reset user passwords |
| users | `users.unlock` | 解锁用户 | Unlock users |
| users | `users.assign_role` | 分配用户角色 | Assign user roles |
| sessions | `sessions.revoke` | 撤销用户会话 | Revoke user sessions |
| roles | `roles.read` | 查看角色 | View roles |
| roles | `roles.create` | 创建角色 | Create roles |
| roles | `roles.update` | 编辑角色 | Edit roles |
| roles | `roles.delete` | 删除角色 | Delete roles |
| audit | `audit.read` | 查看审计日志 | View audit logs |

- 自助操作（自己的显示名、改自己的密码、dashboard）只要求已认证，不进入目录。
- 权限组顺序以代码目录为准；`GET /permissions` 不按数据库顺序。
- 目录新增权限是加法变更；`super_admin` 自动获得未来新增权限；`admin` 的权限集在代码中固定，新增管理权限不会自动授予 `admin`，必须显式修订。

### 6.2 内置角色

| 权限 | `user`（基线） | `admin` | `super_admin` |
|---|---|---|---|
| `users.read` | — | ✓ | 全部（绕过） |
| `users.create` | — | ✓ | 全部 |
| `users.update` | — | ✓ | 全部 |
| `users.set_status` | — | ✓ | 全部 |
| `users.reset_password` | — | ✓ | 全部 |
| `users.unlock` | — | ✓ | 全部 |
| `users.assign_role` | — | ✓ | 全部 |
| `sessions.revoke` | — | ✓ | 全部 |
| `roles.read` | — | ✓ | 全部 |
| `roles.create` | — | — | 全部 |
| `roles.update` | — | — | 全部 |
| `roles.delete` | — | — | 全部 |
| `audit.read` | — | ✓ | 全部 |

- `user` 是每个用户自动拥有且不可移除的基线角色；不授予任何管理权限。
- `admin` 的权限集固定；不包含角色创建/编辑/删除。
- `super_admin` 在授权判定中直接放行，自动拥有当前与未来全部权限；认证与会话有效性仍然适用，step-up 仍然适用。
- 内置角色不可创建、编辑、删除；任何写操作 403 `role_immutable`。

### 6.3 授权规则

1. **多角色并集**：有效权限 = 所有角色权限的并集；v1 不支持 deny、优先级或冲突规则。
2. **即时生效**：角色/权限变更在下一请求生效；RBAC 变更不递增 `token_version`、不撤销会话。
3. **授予子集规则**：非超级管理员创建、修改或分配角色时，目标权限集必须是其自身有效权限的子集；超级管理员豁免。违反返回 403 `permission_subset_violation`。
4. **管理范围规则**：非超级管理员只能读取和管理有效权限不超出自身、且不持有内置 `admin` / `super_admin` 角色的用户。`GET /users` 列表按该范围过滤（`total` 为过滤后数量）；`self` 始终可见/可读；详情与管理动作对范围外目标返回 403 `out_of_scope`。超级管理员不受范围限制。
5. **自我操作禁令**：管理端点不得作用于自己（编辑、启停、重置、角色分配、撤销会话、解锁），返回 403 `self_action_forbidden`；个人资料与改密走 `/auth/me` 自助端点。
6. **特权角色**：`admin` 与 `super_admin` 角色的授予/撤销仅超级管理员；普通管理员在角色选择器中看到但禁用。
7. **最后超管不变量**：系统至少有一个启用的超级管理员；最后一位不能被降级、停用或禁用；禁止自我降级。并发变更用事务行锁校验，违反返回 409 `last_super_admin`。
8. **自定义角色**：id 为 UUIDv7；无 slug；显示名在活跃角色中不区分大小写唯一、可编辑；描述可选；权限至少一项且必须来自目录；不能设置超级管理员标志。
9. **角色软删除**：仅在无任何用户分配（含停用用户）时允许；删除后从列表与分配候选中隐藏、不可再分配，v1 不恢复；记录保留用于审计与历史引用。仍有分配返回 409 `role_in_use`。
10. **权限解析**：v1 每请求从数据库解析，不缓存，fail-closed；`/me` 返回有效权限与角色；服务端始终权威。

### 6.4 `assignable` / `manageable` 目标资格口径

两个标志只表达**目标角色**在授予/管理规则下是否合格，**不包含**操作者是否持有 `users.assign_role`、`roles.update`、`roles.delete`；前端再与 `/me` 权限求交集。API 不新增 `deletable` 或 `assigned_user_count`，不改变 `/roles` 响应形状。

`assignable`：

| 目标角色 | 超级管理员 | 普通管理员 / 其他用户 |
|---|---|---|
| 内置 `user` | true | true（基线） |
| 内置 `admin` / `super_admin` | true | false |
| 自定义角色 | true | `role.permission_keys` 是操作者有效权限子集时为 true |

`manageable`：

| 目标角色 | 超级管理员 | 普通管理员 / 其他用户 |
|---|---|---|
| 内置角色 | false | false |
| 自定义角色 | true | 目标权限集是操作者有效权限子集时为 true |

- `manageable` 不要求操作者持有 `roles.update`；删除额外要求 `roles.delete`，并由服务端校验 `role_in_use`。
- 前端门控：缺少动作权限 → 按钮/菜单项不渲染；有动作权限但目标不可管理 → 按钮禁用 + tooltip（与“无权限”区分）；内置角色永不显示编辑/删除。
- 删除 = `roles.delete` ∧ `manageable` ∧ 无分配（服务端最终裁决）。

### 6.5 端点授权矩阵

| 端点 | 权限 | 附加规则 | step-up |
|---|---|---|---|
| `POST /auth/login` | 无 | Origin + CSRF 头 | 否 |
| `POST /auth/refresh` | refresh Cookie | Origin + CSRF 头 + 单飞 | 否 |
| `POST /auth/logout` | refresh Cookie | 幂等 | 否 |
| `POST /auth/logout-all` | Bearer | 受限会话拒绝 | 否 |
| `GET /auth/me` | 已认证 | 受限会话允许 | 否 |
| `PATCH /auth/me` | 已认证 | 仅 display_name；受限会话拒绝 | 否 |
| `POST /auth/change-password` | 已认证 | 受限会话允许 | 否 |
| `POST /auth/reauthenticate` | 已认证 | 受限会话拒绝 | 否 |
| `GET /users` | `users.read` | 管理范围过滤 | 否 |
| `GET /users/{id}` | `users.read` | 管理范围；self 可见 | 否 |
| `POST /users` | `users.create`（带 `role_ids` 还需 `users.assign_role`） | 授予子集/范围 | 是 |
| `PATCH /users/{id}` | `users.update` | 管理范围；self 禁止 | 含 username/email 时是 |
| `PUT /users/{id}/status` | `users.set_status` | 管理范围；self 禁止；最后超管 | 是 |
| `POST /users/{id}/reset-password` | `users.reset_password` | 管理范围；self 禁止 | 是 |
| `POST /users/{id}/unlock` | `users.unlock` | 管理范围；self 禁止 | 否 |
| `PUT /users/{id}/roles` | `users.assign_role` | 管理范围；授予子集；self 禁止；特权角色仅超管 | 是 |
| `DELETE /users/{id}/sessions` | `sessions.revoke` | 管理范围；self 禁止 | 是 |
| `GET /roles` | `roles.read` | 不按范围隐藏；返回 flags | 否 |
| `GET /roles/{id}` | `roles.read` | 软删除/不存在 404 | 否 |
| `POST /roles` | `roles.create` | 授予子集；内置名保留 | 是 |
| `PATCH /roles/{id}` | `roles.update` | 授予子集；内置不可改 | 是 |
| `PUT /roles/{id}/permissions` | `roles.update` | 授予子集；内置不可改 | 是 |
| `DELETE /roles/{id}` | `roles.delete` | 授予子集；内置不可删；无分配 | 是 |
| `GET /permissions` | `roles.read` 或 `roles.create` 或 `roles.update` 任一 | 返回完整目录 + `grantable` | 否 |
| `GET /audit-logs` | `audit.read` | 不按范围过滤 | 否 |
| `GET /audit-logs/{id}` | `audit.read` | 写 `audit_log_accessed` | 否 |

### 6.6 管理范围与列表行为

- `GET /users` 的 `total` 是**过滤后**数量；列表只返回操作者可管理的用户，`self` 始终可见。
- `GET /users/{id}` 对范围外目标返回 403 `out_of_scope`，不返回 404（避免把“存在但无权”伪装成不存在；二者都不可访问）。
- `GET /roles` 不做范围过滤：普通管理员也能看到 `super_admin` 角色及持有者数量无关的信息；`assignable` / `manageable` 表达其不可操作。
- `GET /audit-logs` 持有 `audit.read` 即可查看全部，不按管理范围过滤。
- 角色删除 409 的“查看使用该角色的用户”链接跳转 `/users?role_id={id}`；该链接仅在同时具备 `users.read` 时显示。

---

## 7. API 契约与错误码

### 7.1 通用约定

- 前缀 `/api/v1`；资源扁平、复数、kebab-case：`/auth/*`、`/users`、`/roles`、`/permissions`、`/audit-logs`。
- 不设 `/admin` 段；管理能力由权限决定，不由 URL 分类。
- 路径参数统一 `{id}`；不允许尾斜杠（直接 404）。
- JSON 字段一律 snake_case；多词路径 kebab-case。
- ID 为小写带连字符 UUID 字符串；时间为 RFC3339 UTC（`...Z`，毫秒精度）；枚举为 snake_case 字符串。
- 响应始终包含字段，缺省用 `null`；请求中省略表示“不修改”，显式 `null` 清空可空字段。
- 写请求拒绝未知字段（400 `unknown_field`）；未知查询参数忽略。
- 创建返回 201 + `Location`；删除/登出/撤销返回 204。
- v1 不引入 `Idempotency-Key`；唯一约束防重复。
- `Cache-Control: no-store` 适用于认证响应（登录、刷新、me、改密、重新认证）。

### 7.2 错误契约（RFC 7807）

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

- `Content-Type: application/problem+json`。
- `type = urn:rs-admin:problem:<code>`；`code` 是稳定扁平 snake_case 标识；`title`/`detail` 英文兜底；服务端不按 `Accept-Language` 变文案，前端按 `code` 映射 zh-CN/en-US。
- `instance` 为请求路径；`request_id` 与响应头 `X-Request-Id` 一致。
- 字段校验用 422 `validation_failed` + `errors[]`（JSON Pointer，支持嵌套；`params` 可选）；JSON 解析失败 400 `invalid_json`；未知请求体字段 400 `unknown_field`。
- 所有 401 带 `WWW-Authenticate: Bearer realm="rs-admin"`；429 带 `Retry-After`；绝不回传内部错误、SQL 或堆栈。
- 4xx/5xx 响应同样带正确的 CORS 头。
- 错误码同步进入 OpenAPI 与前端共享 TS 联合类型；客户端必须容忍未来新增码。

### 7.3 错误码目录

| 类别 | code | HTTP | 语义 |
|---|---|---|---|
| 请求 | `invalid_json` | 400 | JSON 解析失败 |
| 请求 | `unknown_field` | 400 | 请求体含未知字段 |
| 请求 | `unsupported_media_type` | 415 | Content-Type 不支持 |
| 请求 | `method_not_allowed` | 405 | 方法不允许 |
| 请求 | `payload_too_large` | 413 | 请求体超限 |
| 校验 | `validation_failed` | 422 | 字段校验失败（带 `errors[]`） |
| 校验 | `invalid_sort` | 422 | 排序字段不在白名单 |
| 校验 | `page_size_too_large` | 422 | `page_size > 200` |
| 校验 | `empty_update` | 422 | PATCH 空对象 |
| 校验 | `invalid_time_range` | 422 | 审计时间范围格式错误或 `from >= to` |
| 校验 | `time_range_too_large` | 422 | 审计时间跨度 > 90 天 |
| 认证 | `invalid_credentials` | 401 | 统一登录凭据错误 |
| 认证 | `access_token_invalid` | 401 | access token 无效/过期 |
| 认证 | `session_invalid` | 401 | 会话不存在/撤销/过期/版本不符 |
| 认证 | `refresh_token_invalid` | 401 | refresh token 无效/过期 |
| 认证 | `refresh_reuse_detected` | 401 | grace 外刷新重放，整族撤销 |
| 认证 | `account_disabled` | 403 | 账号停用 |
| 认证 | `temporary_password_expired` | 403 | 临时密码过期 |
| 认证 | `login_locked` | 429 | 账号层锁定 |
| 认证 | `rate_limited` | 429 | IP/全局层限流或全局限流 |
| 认证 | `password_change_required` | 403 | 受限会话访问非 allowlist 端点 |
| 认证 | `csrf_failed` | 403 | Origin/CSRF 头校验失败 |
| 认证 | `reauth_required` | 403 | 最近认证超过 300 秒 |
| 认证 | `invalid_current_password` | 422 | 当前密码错误（改密/重新认证） |
| 授权 | `forbidden` | 403 | 缺少权限 |
| 授权 | `out_of_scope` | 403 | 目标超出管理范围 |
| 授权 | `self_action_forbidden` | 403 | 管理动作作用于自己 |
| 授权 | `role_immutable` | 403 | 写内置角色 |
| 授权 | `permission_subset_violation` | 403 | 违反授予子集规则 |
| 资源 | `not_found` | 404 | 资源不存在 |
| 资源 | `identifier_taken` | 409 | username/email 冲突 |
| 资源 | `role_name_taken` | 409 | 活跃角色名称冲突 |
| 资源 | `role_in_use` | 409 | 角色仍有用户分配 |
| 资源 | `last_super_admin` | 409 | 违反最后超管不变量 |
| 校验 | `password_policy_violation` | 422 | 密码策略失败（`params.reason`：`too_short`/`too_long`/`blocklisted`/`equals_identifier`） |
| 校验 | `password_unchanged` | 422 | 新密码等于当前密码 |
| 校验 | `unknown_role` | 422 | 未知/软删除角色 |
| 校验 | `empty_permissions` | 422 | 角色权限为空 |
| 校验 | `invalid_permission` | 422 | 权限不在目录 |
| 服务 | `internal_error` | 500 | 内部错误 |
| 服务 | `service_unavailable` | 503 | 数据库/Argon2 并发不可用 |

状态码语义总表：400 语法/未知字段；401 未认证；403 已认证但禁止；404 资源不存在；405 方法不允许；409 状态冲突；413 请求体过大；415 媒体类型不支持；422 语义/字段校验；429 锁定或限流；500 内部错误；503 服务不可用。

### 7.4 表示、分页、筛选与排序

- 列表统一 offset 分页：`page`（1 起，默认 1）、`page_size`（默认 20，上限 200），响应 `{items, total, page, page_size}`；超过上限返回 422 `page_size_too_large`，不静默截断。
- 后续可按端点差异化上限，但不得超过 200。
- 筛选用每端点显式、有类型的 query 参数，不引入通用 filter DSL。
- 排序用 `sort=field` / `sort=-field`，每端点白名单；字段不在白名单返回 422 `invalid_sort`。
- 列表 envelope 固定；单资源不包 envelope。

### 7.5 共享 schema

- `UserSummary`：`id`、`username`、`email`、`display_name`、`phone`（nullable）、`status`、`roles: RoleRef[]`、`must_change_password`、`locked_until`（nullable）、`last_login_at`（nullable）、`created_at`、`updated_at`。
- `UserDetail`：`UserSummary` + `temporary_password_expires_at`（nullable）；不暴露 `failed_login_count`。
- `UserSelf`：`id`、`username`、`email`、`display_name`、`phone`、`status`、`roles`、`permissions: string[]`、`is_super_admin`、`must_change_password`、`temporary_password_expires_at`。
- `RoleRef`：`id`、`name`、`is_builtin`。
- `RoleSummary` / `RoleDetail`：`id`、`name`、`description`（nullable）、`is_builtin`、`permission_keys: string[]`、`assignable`、`manageable`、`created_at`、`updated_at`。
- `PermissionGroup`：`key`、`display_name_zh`、`display_name_en`、`permissions: PermissionItem[]`。
- `PermissionItem`：`key`、`display_name_zh`、`display_name_en`、`grantable`。
- `AuditLogSummary`：`id`、`occurred_at`、`actor: {id, label}`、`action`、`target: {type, id, label}`、`result`、`severity`、`ip`、`request_id`。
- `AuditLogDetail`：`AuditLogSummary` + `user_agent`、`details`（开放 object）。
- `ProblemDetails`：见 §7.2。

用户校验：

- `username`：3–32、`^[a-z0-9][a-z0-9._-]*$`、保留 `admin`/`root`/`system` 等少量名称。
- `email`：trim + Unicode NFC + 小写、≤254、格式校验。
- `display_name`：1–64 Unicode、缺省 username、不唯一。
- `phone`：≤32、trim、不唯一、不参与登录。
- `password`：15–128 个 Unicode 码点、NFC、blocklist、不截断；输入上限 128 码点 / 512 字节。
- `role name`：自定义角色 1–64、活跃角色中不区分大小写唯一、description ≤500、`permission_keys` 至少一项且来自目录；内置名称保留。

### 7.6 认证端点

| 端点 | 鉴权 | 请求 | 成功 | 主要错误 |
|---|---|---|---|---|
| `POST /auth/login` | 无 | `{identifier, password}` | 200 `{access_token, token_type:"Bearer", expires_in:900, user: UserSelf}` + Set-Cookie refresh | 401 `invalid_credentials`；403 `account_disabled` / `temporary_password_expired`；429 `login_locked` / `rate_limited` |
| `POST /auth/refresh` | refresh Cookie | `{}` | 200 `{access_token, token_type, expires_in}` + 轮换 Set-Cookie | 401 `refresh_token_invalid` / `refresh_reuse_detected` / `session_invalid`；403 `account_disabled` |
| `POST /auth/logout` | refresh Cookie | `{}` | 204 + 清 Cookie（幂等） | 403 `csrf_failed` |
| `POST /auth/logout-all` | Bearer | `{}` | 204；撤销全部会话 + `token_version++` + 清 Cookie | 401 `access_token_invalid`；403 `password_change_required` |
| `GET /auth/me` | Bearer | — | 200 `UserSelf` | 401 `access_token_invalid` / `session_invalid`；403 `account_disabled` |
| `PATCH /auth/me` | Bearer | `{display_name}` | 200 `UserSelf` | 422 `validation_failed` / `empty_update`；403 `password_change_required` |
| `POST /auth/change-password` | Bearer | `{current_password, new_password}` | 200 `{access_token, token_type, expires_in}` + 轮换 refresh Cookie | 422 `invalid_current_password` / `password_policy_violation` / `password_unchanged` |
| `POST /auth/reauthenticate` | Bearer | `{password}` | 200 `{access_token, token_type, expires_in, auth_time}` | 422 `invalid_current_password`；403 `reauth_required` 不在此端点返回（由敏感端点返回） |

- login/refresh/logout/logout-all 要求精确 Origin、`X-Requested-With: XMLHttpRequest`、`Content-Type: application/json`；失败 403 `csrf_failed`。
- `POST /auth/reauthenticate` 不读 refresh Cookie，不要求 CSRF 头。
- 业务 Bearer 端点不读 cookie。
- refresh Cookie 属性：`__Host-refresh; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=43200`，不设 `Domain`。
- login 返回完整 `UserSelf`（含有效权限）；refresh 只返回令牌；`GET /auth/me` 是用户与权限的权威来源。
- 受限会话 allowlist 见 §5.4。

### 7.7 Users 端点

全部要求 Bearer；管理动作对自己 403 `self_action_forbidden`，对范围外目标 403 `out_of_scope`。

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

- 列表默认 `-created_at`；排序白名单 `username` / `email` / `display_name` / `status` / `created_at` / `last_login_at` / `updated_at`。
- `q` 对 username/email/display_name 做大小写不敏感子串搜索（`pg_trgm`）；`role_id` 匹配持有该角色的用户（含基线 `user`）；`is_locked` 按 `locked_until > now()` 派生；`must_change_password` 精确匹配。
- `PATCH /users/{id}` 空对象 422 `empty_update`；username/email 冲突 409 `identifier_taken`；修改 username/email 不撤销会话，写审计。
- 创建/重置统一签发临时密码：省略 `password` 则生成 24 字符 base64url；手输必须通过密码策略；统一 `must_change_password=true`、默认 72h 过期；响应只返回一次；管理员不能对自己重置；重置会撤销目标用户全部会话、`token_version++`、清除账号层锁定。
- 角色分配整体替换：基线 `user` 不可移除；`admin`/`super_admin` 仅超级管理员可授予/撤销；目标权限集必须是操作者有效权限子集，否则 403 `permission_subset_violation`；未知/软删除角色 422 `unknown_role`。
- v1 不提供用户删除、批量操作、邀请流程、管理员永久密码、查看既有密码。

### 7.8 Roles / Permissions 端点

| 端点 | 权限 | 请求要点 | 成功 |
|---|---|---|---|
| `GET /roles` | `roles.read` | `q`、`is_builtin`、`page`、`page_size≤200`、`sort` | 200 列表 `RoleSummary` |
| `GET /roles/{id}` | `roles.read` | — | 200 `RoleDetail`；软删除/不存在 404 |
| `POST /roles` | `roles.create` | `{name, description?, permission_keys}` | 201 + Location `RoleDetail` |
| `PATCH /roles/{id}` | `roles.update` | `{name?, description?}` | 200 `RoleDetail` |
| `PUT /roles/{id}/permissions` | `roles.update` | `{permission_keys}` | 200 `RoleDetail` |
| `DELETE /roles/{id}` | `roles.delete` | — | 204（软删除） |

- 内置角色任何写操作 403 `role_immutable`；自定义角色仅当目标权限集是操作者有效权限子集时可写，否则 403 `permission_subset_violation`；删除时仍有分配（含停用用户）409 `role_in_use`；名称冲突 409 `role_name_taken`；空权限 422 `empty_permissions`；非法权限 422 `invalid_permission`。
- `GET /roles` 默认 `name asc`；排序白名单 `name` / `created_at` / `updated_at`；`q` 搜 name/description；软删除角色从列表隐藏、名称可复用；列表不按管理范围隐藏，返回 `assignable` / `manageable`。
- `GET /permissions` 需要 `roles.read`、`roles.create`、`roles.update` 中任意一项；无分页；返回 `{groups: PermissionGroup[]}`，按代码目录固定顺序；`grantable` 对超级管理员恒 true，其他人按“该权限是否在自身有效权限内”计算。

**角色保存语义**：元数据变更 → `PATCH /roles/{id}`；权限变更 → `PUT /roles/{id}/permissions`；两者都变时先 PATCH 后 PUT（避免名称冲突时白改权限）。不新增合并端点。部分失败（PATCH 成功、PUT 失败）时保留权限 dirty 状态，提示“基本信息已保存；权限未保存”，重新拉取角色；表单保留修改，可直接重试。

### 7.9 Audit 端点

- `GET /audit-logs` 需要 `audit.read`；过滤：`actor_id`、`action`、`target_type`、`target_id`、`result`、`severity`、`request_id`、`ip`、`from`、`to`；默认排序 `-occurred_at`；排序白名单仅 `occurred_at`。
- `GET /audit-logs/{id}` 返回 `AuditLogDetail`；404 `not_found`；每次详情读取写 `audit_log_accessed`（列表查询不写）。
- `from` 含、`to` 不含；未提供时默认最近 30 天；最大跨度 90 天；超限 422 `time_range_too_large`；格式错误或 `from >= to` 422 `invalid_time_range`。
- `ip` 精确匹配：服务端将 IPv4-mapped IPv6 归一化为 IPv4、按标准压缩 IPv6；不做 CIDR。
- 审计日志只读、不按管理范围过滤；CLI 事件 `actor.id=null`、`actor.label="cli"`；系统任务 `actor.label="system"`；未知标识失败登录 `actor.label="anonymous"`。
- `action` 与 `target_type` 在 OpenAPI 中用开放 string + 文档化取值目录（新增事件/目标类型视为加法变更）；`severity` 与 `result` 保持封闭枚举；前端对未知值回退显示原始 code。
- 审计详情 `details` 为开放 object；前端提供结构化/原始 JSON 双视图与精确 key 黑名单脱敏（§9.8）。
- v1 不提供导出、删除、编辑、批量下载、上一条/下一条、链校验按钮。

### 7.10 OpenAPI 与 TypeScript 客户端

- 后端用 utoipa 6 + utoipa-axum 0.3 生成 OpenAPI 3.1；`servers: [{url: "/"}]`，业务端点文档路径带完整 `/api/v1` 前缀（ADR-0004 2026-11-07 修订）；`/api-docs/openapi.json` 仅内网/开发可达；Swagger UI 仅开发/内网启用，生产关闭。
- tags：`Auth`、`Users`、`Roles`、`Permissions`、`Audit Logs`。
- operationId 固定：`login`、`refreshToken`、`logout`、`logoutAll`、`getMe`、`updateMe`、`changePassword`、`reauthenticate`、`listUsers`、`getUser`、`createUser`、`updateUser`、`setUserStatus`、`resetUserPassword`、`unlockUser`、`setUserRoles`、`revokeUserSessions`、`listRoles`、`getRole`、`createRole`、`updateRole`、`setRolePermissions`、`deleteRole`、`listPermissions`、`listAuditLogs`、`getAuditLog`。
- security schemes：`bearerAuth`（HTTP Bearer JWT）与 `refreshCookie`（apiKey cookie `__Host-refresh`，仅文档表达，浏览器自动发送）；`X-Requested-With` 作为 cookie 认证端点的必填 header 参数。
- 统一错误 schema `ProblemDetails`；所有响应文档化 `X-Request-Id`；201 文档化 `Location`；429 文档化 `Retry-After`；401 文档化 `WWW-Authenticate`。
- `openapi.json` 作为构建产物由后端 CI 生成并提交；前端用 orval 8（`client: 'react-query'`、`httpClient: 'axios'`、自定义 mutator 注入 Bearer 并做单飞 401 刷新）生成并提交客户端；CI 用 `git diff --exit-code` 防漂移。
- 生成物提交仓库；不手写业务客户端。

### 7.11 CORS、CSRF 与 Cookie

- CORS：精确 Origin allowlist，不使用通配符，不允许 `null`；`Vary: Origin`；允许方法 `GET/POST/PUT/PATCH/DELETE/OPTIONS`；允许 headers `Authorization, Content-Type, X-Requested-With, X-Request-Id`；暴露 `X-Request-Id, Retry-After, WWW-Authenticate`；`Access-Control-Max-Age: 600`。
- credentials 仅以下端点：`POST /auth/login`、`POST /auth/refresh`、`POST /auth/logout`、`POST /auth/logout-all`、`POST /auth/change-password`。`POST /auth/reauthenticate`、`GET/PATCH /auth/me` 只用 Bearer，不需要 credentials。
- `Access-Control-Allow-Credentials: true` 且 `Access-Control-Allow-Origin` 为具体 origin；4xx/5xx 同样带正确 CORS 头；预检 204。
- CSRF：login/refresh/logout/logout-all 要求 POST + `application/json` + 精确 Origin + `X-Requested-With: XMLHttpRequest`；refresh cookie `SameSite=Strict` 作纵深防御。业务 API 只用 Bearer、不读 cookie；不做 double-submit 或同步令牌。
- 同站兄弟子域仍属 same-site，因此 Origin 精确匹配是主要控制，不能只依赖 SameSite。

---

## 8. 安全控制

### 8.1 密码哈希

- Argon2id v19：`m = 65536 KiB (64 MiB)`、`t = 3`、`p = 4`、输出 32 字节、盐 16 字节；启用 `argon2` crate 的 `parallel` feature。`p` 是 lane 数，不乘内存；不得用提高 `p` 替代提高 `m`。
- 生产基准目标：单次验证 100–300 ms，不得超过 1 秒。若超过 300 ms，按顺序回退：`p = 1` → `m = 32 MiB`；任何情况下不得低于 OWASP 最低档 `m = 19 MiB`、`t = 2`、`p = 1`。
- 存储：`users.password_hash` 保存完整 PHC 字符串（含算法、版本、参数、盐与哈希）；不增加独立 salt/params/version 列。
- 升级：登录验证成功后比较 PHC 参数与当前策略；低于当前策略则用当前策略 rehash 并更新该行；旧哈希在用户下次登录前继续有效；不做强制全员重置。
- 未知标识：执行固定 dummy PHC 验证，参数与当前策略一致；dummy 哈希在启动时用当前参数生成。
- 并发：`PASSWORD_HASH_CONCURRENCY = max(1, min(4, CPU 核数 / p))`；用 `tokio::sync::Semaphore` 限制并发；等待超过 2 秒返回 503 `service_unavailable`（带 `Retry-After`）；内存上限 `64 MiB × 并发数`。
  - 2026-11-11 修订（issue #54）：部署通过 `PASSWORD_HASH_CONCURRENCY=4` 显式覆盖上述公式的默认值，以匹配 §10.10 的 20 并发登录目标与 §1.6 的 256 MiB 内存预算；公式仍是未显式配置时的默认。实测在 2-CPU 容器限制下 20 并发登录的 p95 仍超过 1 秒（`docs/acceptance/performance.md`），该 SLO 按 issue #54 记为风险接受，分析与缓解见 `docs/acceptance/performance-analysis.md`；不得为达标降低 Argon2 强度。
- pepper：v1 不加服务端 pepper。
- 输入上限：密码最多 128 个 Unicode 码点 / 512 字节；认证端点请求体上限 1 KiB。登录密码超过上限时仍执行 dummy 验证并返回 401 `invalid_credentials`；创建/改密超过上限返回 422 `password_policy_violation`。

### 8.2 密码策略

- 长度与复杂度：NFC 规范化后 15–128 个 Unicode 码点；不做大小写/数字/符号组合要求；不做周期性强制改密；v1 不做密码历史。
- 常见/泄漏密码：构建时内嵌离线列表前 100,000 条（HIBP Pwned Passwords 频率列表或等价来源，如 SecLists top-100000，需兼容许可证）；列表版本化、随发布更新、不联网查询。密码经 NFC + 小写后与列表精确匹配；命中返回 422 `password_policy_violation`，文案不透露具体来源。
- 标识关联：拒绝与 username、email 或 display_name（NFC + 小写后）完全相同的密码。
- 规范化：密码不 trim、不截断；NFC 后原样交给 Argon2；长度按码点计。
- 修改密码：自助改密要求当前密码与新密码；新密码不得等于当前密码（422 `password_unchanged`）；成功后按 §5.7 保留当前会话、撤销其他会话、`token_version++`、重签 access token。
- 临时密码：系统生成 24 字符 base64url（约 144-bit）；管理员手输的临时密码同样必须通过上述策略；默认 72 小时过期；`must_change_password = true`；密码值永不写日志或审计。
- 错误码：`password_policy_violation`（稳定内部 reason：`too_short`/`too_long`/`blocklisted`/`equals_identifier`）、`invalid_current_password`、`password_unchanged`。

### 8.3 登录限流与账号锁定

限流状态全部存 PostgreSQL，不使用进程内存作为唯一计数；所有窗口判断使用数据库 `now()`。

**账号层**

- 已知用户：`users.failed_login_count`、`users.locked_until`、`users.last_failed_login_at`。
- 未知标识：`auth_login_failures.key_hash = HMAC-SHA256(THROTTLE_KEY, 规范化标识)`、`failed_count`、`locked_until`、`updated_at`。
- 阈值与退避：5 次失败锁 30 秒、10 次锁 5 分钟、15 次锁 1 小时；计数在锁定到期后保留，成功登录清零；连续 24 小时无失败自动衰减为 0；计数 ≥15 时每次新失败继续锁 1 小时。不做永久自动锁定。
- 锁定时仍执行一次 dummy Argon2 验证，返回 429 `login_locked` + `Retry-After`；锁定期间不增加计数。密码正确但用户停用或临时密码过期返回 403（`account_disabled`/`temporary_password_expired`），不增加失败计数。
- 管理员解锁（`users.unlock`）清除已知用户的账号层计数、`last_failed_login_at` 与锁定，不动 IP/全局计数；管理员重置凭据同样清除账号层计数、`last_failed_login_at` 与锁定。

**IP 层**

- 表：`auth_login_throttle(scope, key_hash, window_start, failures, locked_until, last_failure_at)`；`scope IN ('ip','global')`；`key_hash = HMAC-SHA256(THROTTLE_KEY, 'ip:' || 规范化 IP)`。
- IPv4 精确匹配；IPv6 按 /64 聚合；未知与已知标识都计数。
- 阈值：20 次失败 / 10 分钟固定窗口 → 锁 15 分钟。成功登录不重置 IP 计数。
- 锁定时不执行 Argon2，直接 429 `rate_limited` + `Retry-After`；锁定期间不增加计数。

**全局层**

- 同一张表，单行 `scope = 'global'`，`key_hash = HMAC-SHA256(THROTTLE_KEY, 'global')`。
- 告警阈值：200 次失败 / 5 分钟；熔断阈值：1000 次失败 / 5 分钟 → 全站登录/重新认证 429 `rate_limited` 60 秒。阈值可配置。
- 熔断只作用于登录与重新认证，不影响 refresh、业务 API 或已登录会话。
- 锁定时不执行 Argon2；锁定期间不增加计数。触发与解除均告警。

**实现约束**

- 计数写入使用 `INSERT ... ON CONFLICT DO UPDATE ... RETURNING`；禁止 `SELECT count(*)` 后插入。
- advisory lock 只包住计数/锁定更新，不包 Argon2 验证（避免串行化登录、放大 DoS）。
- 数据库不可用时认证 fail-closed（503）。
- 清理：`auth_login_throttle` 与 `auth_login_failures` 中超过 30 天且未锁定的行由运维任务批量删除；为 `last_failure_at`/`updated_at` 建索引；监控表大小与清理延迟。
- 不做 IP + 账号组合键；v1 无 CAPTCHA。
- 重新认证的密码失败计入账号层与 IP 层计数；自助改密当前密码错误同样计入（§5.7）。
- refresh 端点不做每会话计数；由网关对 `/auth/*` 做每 IP 60 次/分钟的粗粒度限制。

### 8.4 可信代理与客户端 IP

- 配置 `TRUSTED_PROXY_CIDRS`（逗号分隔 CIDR）。为空时忽略 `X-Forwarded-For`，只使用 TCP peer 地址。
- peer 在可信列表内时，从右向左解析 `X-Forwarded-For`，跳过可信代理，取第一个不可信地址作为客户端 IP；全部可信或格式非法时回退到 peer 并记录告警。
- 只认 `X-Forwarded-For`；忽略 `X-Real-IP` 与 `Forwarded`。网关必须覆盖而非追加该头。
- IPv4-mapped IPv6 归一化为 IPv4；IP 限流键使用 IPv4 地址或 IPv6 /64；审计保存规范化后的原始 IP。
- 限流表只保存 `HMAC-SHA256(THROTTLE_KEY, 规范化键)`，不保存明文 IP；审计表保存原始 IP。
- 生产环境在网关后运行时 `TRUSTED_PROXY_CIDRS` 不应为空；启动时检查并告警（不直接失败，允许直连部署）。

### 8.5 审计事件、字段与防篡改

#### 8.5.1 事件目录

| action | severity | 触发 |
|---|---|---|
| `authn_login_success` | info | 登录成功（含受限会话） |
| `authn_login_fail` | warning | 密码错误、未知标识、停用、临时密码过期 |
| `authn_login_lock` | warning | 账号层达到锁定阈值（登录、重新认证或自助改密） |
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
| `user_password_reset` | critical | 管理员重置凭据（`method=generated\|manual`） |
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

#### 8.5.2 字段与 target 映射

- 顶层字段：`id`、`occurred_at`、`actor_user_id`、`actor_label`、`action`、`target_type`、`target_id`、`target_label`、`result`、`severity`、`ip`、`user_agent`、`request_id`、`details`。
- `result` 取值 `success`、`failure`、`denied`：成功动作为 `success`；认证/操作失败为 `failure`；权限、step-up 或账号状态拒绝为 `denied`。
- `actor_label`：用户动作为事件发生时的 username 快照；CLI 为 `cli`；系统任务为 `system`；未知标识的失败登录为 `anonymous`。
- `request_id` 即 trace_id；不新增 `trace_id` 列。
- 密码、密码哈希、临时密码、access/refresh token、Cookie、Authorization 头、密钥永不出现在审计中。
- `ip` 为可信代理解析后的规范化地址；`user_agent` 截断到 512 字符；`details` 不包含请求体或 query string。
- target 映射：

| 事件组 | `target_type` | `target_id` | `target_label` |
|---|---|---|---|
| `user_*`、`authn_password_changed`/`_fail`、`authn_reauth_*`、`system_bootstrap_super_admin`/`system_recover_super_admin` | `user` | 目标用户 id | username 快照 |
| `authn_login_success`、`authn_logout`、`authn_refresh_reuse_detected`、`authn_session_evicted` | `session` | 会话 id | null |
| `authn_logout_all` | `user` | 用户 id | username 快照 |
| `authn_login_fail` / `authn_login_lock`（已知用户） | `user` | 用户 id | username 快照 |
| `authn_login_fail`（未知标识）、`authn_login_rate_limited`（IP/全局） | `system` | null | null |
| `authz_denied` / `authz_step_up_denied` | 能确定被拒对象时 `user`/`role`，否则 `system` | 对应 id 或 null | 对应名称快照或 null |
| `role_*` | `role` | 角色 id | name 快照 |
| `crypto_key_rotated`、`system_audit_logs_purged` | `system` | null | null |
| `audit_log_accessed` | `audit_log` | 被查看日志 id | null |
| 未来未知事件 | 可空或 `system` | 可空 | null |

- `target_type` 当前目录为 `user`、`role`、`session`、`audit_log`、`system`，允许未来新增；前端与验证工具对未知值按原始字符串处理。
- `details` 按事件定义，至少包含：
  - 登录失败：`reason_code`（`bad_credentials`/`account_disabled`/`temporary_password_expired`）；未知标识附 `identifier_hmac`。
  - 登录成功/重新认证：`auth_method=password`、`session_id`、`must_change_password`（登录）；`auth_time`（重新认证）。
  - 登出：`session_id`；全部登出：`sessions_revoked_count`。
  - 锁定/限流：`lock_seconds`、`failure_count`、`source`（`login`/`reauth`/`change_password`）或 `scope`（`ip`/`global`）、`retry_after_seconds`。
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

#### 8.5.3 存储与链哈希

- 写入路径：在业务事务内执行 `SELECT last_seq, last_hash FROM audit_chain_head WHERE id = 1 FOR UPDATE`；应用生成 UUIDv7 `id`，使用数据库 `now()` 作为 `occurred_at`；`chain_seq = last_seq + 1`；`prev_hash = last_hash`；`row_hash = HMAC-SHA256(AUDIT_CHAIN_KEYS[hmac_key_id], prev_hash || 0x00 || JCS(row_without_row_hash))`，其中 `JCS` 为 RFC 8785 规范化 JSON，`row` 包含 `hmac_key_id`；插入审计行并更新链表头。事务提交前不得释放锁。
- 规范化时 UUID、时间、`inet` 用字符串，`prev_hash` 用小写十六进制，`details` 用 JCS，数值按 JCS 规则；验证工具按同一规则重算。
- 审计写入与业务动作同一事务，fail-closed：审计写失败则业务动作失败（500/503）并告警。登录失败等无业务事务的事件同样在返回前写审计；写失败返回 503。
- 运行角色对 `audit_logs` 只有 `INSERT`/`SELECT`，无 `UPDATE`/`DELETE`；对 `audit_chain_head` 有 `SELECT`/`UPDATE`。清理任务用独立角色，按保留期批量 `DELETE`，并在同一链路写入 `system_audit_logs_purged`，`details` 记录被删区间与最后一行的 `row_hash` 作为验证锚点。
- 保留期：`info`/`warning` 180 天；`error`/`critical` 365 天；可配置且不得低于该下限。清理批量执行、限制单次删除行数，并监控延迟。
- 外部隔离存储/WORM/日志外发不在 v1 范围；v1 的 append-only 权限 + HMAC 链只能检测数据库层篡改，不能抵御同时掌握应用密钥与数据库的攻击者，残余风险写入安全评审。
- 告警：刷新复用、锁定激增、管理员角色变更、密钥轮换失败、审计写入失败、异常地理位置/设备登录；审计访问本身由 `audit_log_accessed` 记录。

### 8.6 安全响应头、CORS 与 CSP

**API 响应头**（所有 `/api/v1` 响应）：

```text
Cache-Control: no-store
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
X-Frame-Options: DENY
Content-Security-Policy: default-src 'none'; frame-ancestors 'none'; base-uri 'none'
Permissions-Policy: camera=(), microphone=(), geolocation=()
```

**网关/TLS 终止层**：`Strict-Transport-Security: max-age=31536000; includeSubDomains`；`preload` 待实际域名确认；HTTP 自动跳转 HTTPS。

**前端静态站 CSP**：

```text
default-src 'self';
script-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' data: blob:;
font-src 'self' data:;
connect-src 'self' https://api.example.com;
object-src 'none';
base-uri 'self';
form-action 'self';
frame-ancestors 'none';
upgrade-insecure-requests
```

- Vite 构建关闭 modulepreload polyfill 或改用 hash，确保无内联脚本。
- `style-src 'unsafe-inline'` 是 Radix/shadcn 内联样式的现实需要；不得放宽 `script-src`。
- CSP 由网关或 `web` 统一输出，二选一但只能有一处，避免重复头。

**CORS/CSRF**：见 §7.11。

### 8.7 密钥与配置管理

- 交付方式：生产优先读取挂载的 secret 文件（`*_FILE`），环境变量作为回退；不引入 Vault 客户端，具体 secret manager 由部署实现决定；v1 使用主机文件 + Compose secrets。
- 必需配置：
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
- 启动校验：文件存在且可读；PEM 可解析且算法为 Ed25519；`kid`/`iss`/`aud` 非空；所有 HMAC/派生密钥 ≥32 字节；CIDR、Origin 可解析且 Origin 为 `https://` 绝对地址、无通配符/`null`；审计链密钥环包含当前 `key_id`；数据库可连接且使用 `verify-full`。失败即退出，错误信息不包含密钥值。
- 轮换：
  - JWT：publish-before-use；先发布新公钥并等待至少 1 小时，再切换签名；旧公钥保留 ≥ access token TTL + 时钟偏差（≥1 小时）。
  - refresh 派生密钥：新钥上线后旧钥保留 ≥5 分钟（覆盖 30 秒 grace）。
  - throttle 密钥：轮换会重置 IP/全局计数与未知标识的账号计数，安排在低峰期。
  - 审计链密钥：新 `key_id` 写入 `hmac_key_id`；旧钥在 `AUDIT_CHAIN_KEYS_FILE` 中保留 ≥365 天以验证历史记录。
  - 每次轮换写 `crypto_key_rotated`。
- JWT 校验：固定 EdDSA、`kid` 从本地可信密钥集选择、`iss`/`aud` 必填、`exp`/`nbf` 启用、时钟偏差 60 秒、显式 `typ=at+jwt`；不信任 token 中的 `jku`/`jwk`/`x5u`。v1 不发布 JWKS。
- 开发环境：提供 `.env.example` 占位与密钥生成脚本；`.env` 与真实密钥 gitignored；生产不使用明文 `.env`。

### 8.8 日志与脱敏

- 结构化 JSON 输出到 stdout；`tracing` + `tracing-subscriber`；`EnvFilter::try_from_default_env()`，非法指令启动失败。
- 应用日志字段白名单：`request_id`、`method`、`matched_path`、`status`、`latency_ms`、`user_id`、`error_code`。不记录 query string、请求/响应体、headers。
- 永不记录：密码、密码哈希、临时密码、access/refresh token、`Authorization`、`Cookie`、`Set-Cookie`、MFA/重置密钥、审计链密钥、数据库 URL。敏感值用 `secrecy` 等类型包裹，`#[tracing::instrument(skip(...))]`，`TraceLayer` 不启用 `include_headers`。
- username/email 不进入应用日志（用 `user_id`）；IP/User-Agent 只进入审计。错误日志只记录稳定错误码与内部 reason，不记录 SQL、堆栈或请求体。
- `request_id` 由服务端生成 UUIDv7，忽略客户端传入；通过 `X-Request-Id` 回传；应用日志与审计共用该值作为 trace_id。
- metrics 标签使用 axum `MatchedPath` 与有限 reason 枚举（如 `bad_credentials`/`locked`/`rate_limited`），不带 username/email/IP。
- 生产环境关闭 SQLx 的 INFO 查询日志，禁止记录查询参数。
- 应用日志保留 14 天（Loki），由部署执行；前端令牌不得写入 localStorage/console；错误上报需脱敏。

### 8.9 有意偏离与残余风险

- **持久 refresh cookie**：`Max-Age=43200` 是对 NIST SP 800-63B-4 “bearer secret 不应跨浏览器重启持久化”的有意偏离；用 idle 30 分钟、absolute 12 小时、轮换、复用检测与 `__Host-` 属性限制风险。必须在安全评审记录。
- **管理员知道临时密码**：系统无邮件服务，管理员必须线下交付临时密码；这是对 ASVS 6.4.6 的有意偏离。缓解：一律临时密码、72 小时过期、首次登录强制改密、会话撤销、完整审计、管理员不能查看既有密码或再次读取已签发临时密码。
- **审计链的边界**：append-only 权限 + HMAC 链不能抵御同时掌握应用密钥与数据库的攻击者；v1 无 WORM。残余风险写入安全评审。
- **单机部署**：99.5% 依赖主机稳定、外部 uptime 与异地备份；RTO ≤ 2 小时是目标而非保证。

---

## 9. 前端规格

### 9.1 最终页面清单与裁剪

保留：

| 路由 | 页面 / 用途 | 权限 |
|---|---|---|
| `/` | Dashboard 占位壳 | 仅登录；不接真实业务数据 |
| `/users` | Users | `users.read` |
| `/roles` | 角色列表 | `roles.read` |
| `/roles/new` | 创建角色 | `roles.create` |
| `/roles/$roleId` | 角色详情/编辑 | `roles.read` |
| `/audit-logs` | 审计日志查询 | `audit.read` |
| `/settings` | Settings 布局 | 仅登录 |
| `/settings/` | Profile | 仅登录 |
| `/settings/account` | Account | 仅登录 |
| `/settings/appearance` | Appearance | 仅登录 |
| `/settings/notifications` | Notifications | 仅登录；本地偏好 |
| `/settings/display` | Display | 仅登录；本地偏好 |
| `/sign-in` | 登录 | 公开 |
| `/change-password` | 强制改密 / 自助改密 | 需要认证 |
| `/401`、`/403`、`/404`、`/500`、`/503` | 错误页 | 公开，不套认证守卫 |

删除：`tasks`、`apps`、`chats`、`help-center`、`sign-in-2`、`sign-up`、`forgot-password`、`otp`、`clerk/**`、`/_authenticated/errors/$error` 错误演示容器。保留 `/403` 与 `forbidden-error` 组件（研究审计曾建议删除，但角色/认证票据要求真实 403 页面；本规格以保留为准）。

裁剪细则：

- **Dashboard**：保留路由与受保护占位壳；移除假指标、假销售记录、随机图表与 `Recent Sales` 头像引用；占位内容只显示本地化欢迎语与按权限可见的快捷入口；删除 `recharts`。
- **TeamSwitcher**：删除；v1 无多团队/多租户，`sidebarData.teams` 一并移除。
- **Users 假数据**：删除 `@faker-js/faker`、`src/features/users/data/users.ts` 与相关 fixture；保留路由、共享 data-table 组件与空态/骨架；测试 fixture 改为固定对象/MSW。
- **上游残留 UI**：删除 TopNav Customers/Products 禁用链接、`nav-user`/`profile-dropdown` 的 Billing/Upgrade to Pro/New Team、社交登录按钮、无效 Terms/Privacy 链接、`src/assets/brand-icons/**`、Clerk logo/品牌素材、`src/components/learn-more.tsx`、`src/components/coming-soon.tsx`、`src/components/ui/input-otp.tsx`。删除依赖 `@clerk/react`、`input-otp`、`@faker-js/faker`、`recharts`；保留 `date-fns`、`react-day-picker`、`cmdk`。
- **品牌与许可**：界面名称、`index.html` 标题与文档标题使用 `rs-admin`；移除上游 logo/名称与 Clerk 赞助徽章；保留上游 `LICENSE` 原文（MIT，`Copyright (c) 2024 Sat Naing`）；新增 `THIRD-PARTY-NOTICES.md`；README 注明 “Based on satnaing/shadcn-admin (MIT), snapshot e16c87f”。
- **调试辅助**：`showSubmittedData` 不再作为生产提交路径；如保留仅限开发调试。
- **占位壳**：不能伪造业务数据；空态需说明真实数据将在后续实现接入。

### 9.2 路由树、侧栏与命令菜单

```text
src/routes/
├── __root.tsx                         # 全局错误边界、notFound、DocumentTitle
├── (auth)/
│   ├── route.tsx                      # AuthLayout（居中卡片）
│   ├── sign-in.tsx
│   └── change-password.tsx
├── (errors)/
│   ├── 401.tsx / 403.tsx / 404.tsx / 500.tsx / 503.tsx
├── _authenticated/
│   ├── route.tsx                      # bootstrap + 登录/强制改密/权限守卫 + 应用外壳
│   ├── index.tsx                      # Dashboard
│   ├── users/index.tsx
│   ├── roles/index.tsx
│   ├── roles/new.tsx
│   ├── roles/$roleId.tsx
│   ├── audit-logs/index.tsx
│   └── settings/
│       ├── route.tsx
│       ├── index.tsx / account.tsx / appearance.tsx / notifications.tsx / display.tsx
```

- `/change-password` 放在 `(auth)` 分组，使用认证布局，不套应用侧栏外壳；其 `beforeLoad` 调用共享认证 helper（`bootstrap()` + 未登录重定向）；已认证但 `must_change_password=false` 时也允许自助改密。`_authenticated` 守卫把受限会话重定向到 `/change-password?redirect=<原路径>`；`/change-password` 自身不会触发循环重定向。
- `/sign-in` 已认证时跳转净化后的 `redirect` 或 `/`；未认证时正常渲染登录页；`beforeLoad` 只做已认证判断，不要求业务权限。
- 公开错误页不套认证守卫；`/403` 由权限守卫跳转 `?from=<原路径>`，页面提供返回与联系管理员说明，不提供“切换账号”按钮。
- 侧栏分组与顺序：
  - Dashboard（单项，无需业务权限）。
  - Management：Users → Roles → Audit Logs；分别按 `users.read`、`roles.read`、`audit.read` 显示，缺少权限时隐藏。
  - Settings：Profile、Account、Appearance、Notifications、Display；仅要求登录。
- 侧栏数据从模块级常量改为 `useSidebarData()`（携带 `titleKey` 与可选 `permission`）和 `useVisibleSidebarData()`（按 `/me.permissions` 与 `is_super_admin` 过滤）；`CommandMenu` 消费同一份过滤数据；删除的页面不得出现在侧栏、命令菜单、面包屑或 profile 菜单中。
- 命令菜单保留主题命令（Light/Dark/System），不新增语言命令；语言切换只在 Settings/Account。
- 路由权限映射集中在 `route-permissions` 配置；页面不得重复判断。每个路由声明 `staticData.titleKey`，供文档标题与面包屑使用；错误页使用自己的标题 key。

### 9.3 认证状态、冷启动与 API client

- Zustand `auth-store` 只保存 `accessToken` 与 `authStatus`；**不使用 `persist`**；不写 localStorage、sessionStorage 或 JS 可读 Cookie。
- 当前用户与权限由 TanStack Query 的 `['auth','me']` 持有，数据来自 `UserSelf`；登录响应里的 `user` 直接写入该 query；refresh 响应不包含用户。
- 根路由 `beforeLoad` 统一执行 `bootstrap()`（单飞）：
  1. 若已有内存 token 与 `['auth','me']`，直接复用；
  2. 否则 `POST /auth/refresh`（`withCredentials` + CSRF 头）；
  3. 成功后保存 access token，再 `GET /auth/me` 写入 query cache；
  4. refresh 返回 401 时静默置为未登录；
  5. refresh 或 `/me` 遇到网络/5xx 时不误判为登出，保持 `unknown` 并允许重试；公开路由仍可渲染，受保护路由显示可重试的引导错误；
  6. refresh 返回 403 `account_disabled` 时清空会话并回登录页提示账号已停用。
- 登录成功、登出、刷新失败时清空 query cache，避免跨用户残留数据。
- API client：orval 8 生成的 React Query hooks + axios custom mutator；注入 Bearer、统一 `ApiError`、处理 401 刷新与错误；生成物提交，CI 防漂移。
- `VITE_API_BASE_URL`：生产占位 `https://api.example.com/api/v1`；开发默认 `/api/v1`，由 Vite dev proxy 转发到后端，避免本地跨源 Cookie/CORS 噪音。
- axios 默认 `withCredentials: false`；仅以下端点 `withCredentials: true`：`POST /auth/login`、`/auth/refresh`、`/auth/logout`、`/auth/logout-all`、`/auth/change-password`。
- `POST /auth/login`、`/auth/refresh`、`/auth/logout`、`/auth/logout-all` 发送 `X-Requested-With: XMLHttpRequest` 与 `Content-Type: application/json`；`Origin` 由浏览器自动携带；`POST /auth/reauthenticate` 不发送 CSRF 头。
- `ApiError` 保留 `status`、`code`、`request_id`、`errors[].pointer/code/params`、`Retry-After`；非 ProblemDetails 响应（网关 HTML、代理错误）回退为按状态码分类的通用错误；上游 `handleServerError` 对 `{title}` 的假设被替换。
- 若 Bun/Node 版本实测无法运行 orval 8.46，按 ADR-0004 回退 `openapi-typescript` + `openapi-fetch`；mutator 的认证、错误与刷新契约保持不变。

### 9.4 401 刷新、并发去重与重试

- 响应拦截器维护模块级 `refreshPromise`：并发 401 共享同一次 `POST /auth/refresh`，只触发一次令牌轮换。
- 业务请求收到 401（`access_token_invalid` / `session_invalid`）时：
  1. 若该请求已重试过，直接失败；
  2. 否则等待 `refreshPromise`；
  3. 成功后更新内存 token，并**只重试原请求一次**。
- 不触发 401 刷新的端点：`POST /auth/login`、`POST /auth/refresh`、`POST /auth/reauthenticate`、`POST /auth/logout`。`GET/PATCH /auth/me`、`POST /auth/change-password`、`POST /auth/logout-all` 仍可在 401 后刷新并重试一次。
- 刷新失败（401 `refresh_token_invalid` / `refresh_reuse_detected`、403 `account_disabled`、网络错误或 5xx）时：清空 auth store 与 query cache；若当前不在 `/sign-in`，跳转 `/sign-in?redirect=<当前站内路径>`；只提示一次“会话已过期”；403 `account_disabled` 提示账号已停用。
- 移除上游 `QueryCache` 的 401 处理，避免与拦截器重复 toast/跳转；401、403、429、500、503 的错误展示收敛到 API client 与全局错误映射。
- 不做按 `expires_in` 的定时主动刷新；刷新由冷启动与 401 被动触发。
- 查询默认值：401/403/429 永不重试；网络错误与 5xx 的只读查询最多重试 1 次；mutation 不自动重试。401 后刷新再重试原请求是安全的，因为请求在鉴权阶段已被拒绝。

### 9.5 路由守卫、403 与受限会话

- `_authenticated` 根路由的 `beforeLoad` 按顺序执行：
  1. `bootstrap()`；
  2. 未认证 → `redirect('/sign-in', { redirect: 当前站内路径 })`；
  3. `must_change_password === true` 且目标不是 `/change-password` → `redirect('/change-password', { redirect: 当前站内路径 })`；
  4. 路由声明的权限检查。
- `redirect` 参数只接受站内相对路径：必须以单个 `/` 开头，拒绝 `//`、反斜杠、协议/主机、控制字符，以及 `/sign-in` 自循环；无法解析时回退 `/`。
- 权限不足时跳转到真实 `/403?from=<原路径>` 页面；不显示空列表、不静默重定向。
- 权限映射：
  - `/`、`/settings/*`、`/change-password`：仅要求登录；
  - `/users`：`users.read`；
  - `/roles`、`/roles/$roleId`：`roles.read`；
  - `/roles/new`：`roles.create`；
  - `/audit-logs`：`audit.read`。
- 业务 API 返回 403 `password_change_required` 时跳转 `/change-password`；返回 403 `account_disabled` 时清空会话并回登录页提示；403 `reauth_required` 由重新认证 helper 处理，不当作权限不足或 401。
- 受限会话只能先改密：除 `/change-password`、`/401`、`/403`、`/404`、`/500`、`/503`、`/sign-in` 外，其他路由重定向到 `/change-password`。

### 9.6 登录、登出与错误体验

- 登录表单只保留「标识（username 或 email）+ 密码」；移除 sign-up、forgot-password、OTP、社交登录按钮与无效 Terms/Privacy 链接。
- 错误按稳定 `code` 映射，不使用后端 `title`/`detail` 作为主要文案：
  - 401 `invalid_credentials`：统一的「标识或密码错误」，不区分用户是否存在；
  - 403 `account_disabled`：账号已停用；
  - 403 `temporary_password_expired`：临时密码已过期，需要管理员重置；
  - 429 `login_locked` / `rate_limited`：读取 `Retry-After` 倒计时并禁用提交；
  - 422：字段级错误；
  - 网络/5xx：通用连接或服务错误，显示 `request_id` 便于排查。
- 登录成功：清空旧 query cache → 保存 access token → 写入 `['auth','me']` → 按净化后的 `redirect` 跳转；若 `must_change_password` 为 true，跳 `/change-password?redirect=<原目标>`。
- 登出：确认框 → `POST /auth/logout`（带 CSRF 头）→ **无论请求成功与否**清空 auth store 与 query cache → 跳 `/sign-in`；请求失败时额外提示「本机已退出，服务器会话可能仍有效」。`POST /auth/logout-all` 类似，失败时明确提示未完成全部撤销。
- 多标签页只广播登出（`BroadcastChannel('rs-admin-auth')`）：其他标签页清空状态；位于受保护路由时跳登录，公开页只清状态。access token 不跨标签共享；刷新令牌轮换由服务端 grace 窗口兜底。
- 登录、登出都不写入分析日志或持久化存储；密码字段不进入 query cache。

### 9.7 权限门控与重新认证

- `useAuth()` 从 `['auth','me']` 读取用户；`hasPermission(key)` 在 `is_super_admin === true` 时恒真，否则检查 `permissions.includes(key)`。
- 侧栏 nav item 增加可选 `permission` 字段，`useVisibleSidebarData()` 过滤；缺少权限时**隐藏**该项；命令菜单消费同一份过滤数据。
- 目标级不可操作（`assignable` / `manageable` 为 false）才使用禁用 + tooltip，与“无权限”区分。
- 操作按钮使用 `<Can permission="...">` 或 `usePermission`；缺少动作权限时隐藏，目标级不可操作时禁用并说明原因。服务端仍是最终裁决。
- 路由权限映射集中在 `route-permissions` 配置，不在各页面重复判断；直接访问无权限路由显示 403。
- `['auth','me']` 在登录、`PATCH /auth/me` 成功、改密成功后更新/失效；生产环境开启 `refetchOnWindowFocus`，`staleTime` 30 秒，不轮询。收到 `forbidden` / `out_of_scope` 时可失效一次 `['auth','me']` 以纠正过期权限，但不自动跳转。
- 导航门控只依赖 `/me.permissions`；不在启动时为了菜单额外拉 `GET /permissions`。权限目录的完整清单仍用于角色矩阵等管理页面。

**共享重新认证 helper**

- 全局 `ReauthProvider` 与 Promise 式对话框；`requestReauth()` 单飞：并发 403 共享同一个弹窗与同一次 `POST /auth/reauthenticate`。
- `runWithReauth(fn)` 捕获 `ApiError.code === 'reauth_required'`：
  1. 若已有重新认证在进行，等待它；
  2. 否则打开对话框；成功后更新内存 access token；
  3. **重试 `fn()` 一次**；第二次仍 `reauth_required` 则直接抛出，不循环。
- 取消对话框时 reject `ReauthCancelledError`；调用方保留表单状态、不执行原操作。密码错误 422 `invalid_current_password` 在对话框内联提示；429 显示 `Retry-After` 倒计时；403 `password_change_required` 关闭对话框并跳 `/change-password`。
- 重新认证成功后的 5 秒内再次收到 `reauth_required`（通常是认证前已发出的并发请求）直接重试、不再弹窗，用于吸收竞态。**不解码** access token 的 `auth_time` 做主动预判；服务端 `auth_sessions.authenticated_at` 始终权威。
- 角色保存的 PATCH + PUT 共享一次 step-up：顺序执行时第一个请求触发弹窗，第二个在 300 秒窗口内直接通过；并发时单飞弹窗 + 5 秒竞态窗口保证不二次弹窗。
- 只有可能要求 step-up 的 mutation 使用 `runWithReauth`；GET 请求与登录/刷新/登出/重新认证本身不包装。

### 9.8 Users 页面

**信息架构与数据**

- `/users` 保留列表路由，要求 `users.read`；创建、编辑使用对话框；详情使用 URL 同步的只读右侧 Sheet（`/users?user=<id>`），关闭时清除 `user`；不新增 `/users/$userId` 路由。
- 行点击打开详情 Sheet；行操作菜单阻止冒泡；创建成功不自动跳转详情。
- 详情 Sheet 用 `GET /users/{id}` 拉取 `UserDetail`；列表使用 `UserSummary`；不引入头像。
- 详情内容：头部 username/display_name/状态与标记；账号 id（可复制）、username、email、phone；状态 status、locked_until、must_change_password、temporary_password_expires_at；角色 chips；时间 last_login_at、created_at、updated_at；不显示任何密码值。
- Sheet 操作：编辑、分配角色、重置密码、解锁（仅锁定时）、停用/启用、撤销全部会话；全部按权限隐藏、对自己禁用；操作成功后 Sheet 保持打开并重新拉取。v1 不提供跨页“查看审计日志”链接。
- 页面只提供“创建用户”按钮，不提供邀请/邮件流程；创建按钮按 `users.create` 显示。

**列表数据、列与 URL 状态**

- TanStack Query + 服务端 data-table：`manualPagination`、`manualFiltering`、`manualSorting`；数据来自 `GET /users`。
- URL search 直接采用后端参数名：`page`（1 起，默认 1）、`page_size`（默认 20，选项 10/20/50/100，上限 200）、`q`、`status`、`role_id`、`is_locked`、`must_change_password`、`sort`（默认 `-created_at`）、`user`（详情 Sheet）。
- URL search 用 Zod 校验；非法值丢弃并回退默认；`page_size > 200` 不静默截断，回退默认；页码越界自动回到最后一页。
- 筛选、排序变化重置 `page=1` 并关闭详情 Sheet；列表状态变更用 `replace` 导航，详情 Sheet 开关用 `push`，返回键可关闭详情；列表 query 使用 `keepPreviousData`。
- 默认列：username（sticky、不可隐藏、可排序）、display_name、email、roles、status、last_login_at、created_at、actions；phone 与 updated_at 默认隐藏，可用列切换打开；列可见性只保存在组件内存。
- 移除上游多选列；v1 无批量操作。
- roles 列使用 chips：内置角色显示 `roles.builtin.<code>` 本地化名称 + 稳定 code chip；自定义角色原样显示；溢出显示 `+N`，可展开。
- status 单元格展示 active/disabled 徽章，并叠加“已锁定”（`locked_until > now()`）与“待改密”（`must_change_password`）标记；状态筛选只提供 active/disabled。
- `last_login_at=null` 显示“从未登录”；日期/数字使用 `src/lib/format.ts` 的 `Intl` helper，按浏览器本地时区展示，tooltip/复制提供 RFC3339 UTC。
- 排序白名单仅 `username`/`email`/`display_name`/`status`/`created_at`/`last_login_at`/`updated_at`；roles 列不排序。
- 显示 `total`（过滤后数量）；actions 列在所有动作权限都缺失时隐藏，行点击仍可打开详情。

**筛选与深链**

- 主筛选行：`q`（300ms 防抖，服务端对 username/email/display_name 做大小写不敏感子串搜索）、status、角色筛选、排序。
- “更多筛选” popover：`is_locked` 与 `must_change_password` 三态（全部/是/否）。
- 已启用筛选显示为可单独移除的 chips，并提供“清除全部”。
- 角色筛选下拉需要 `roles.read` 才能拉取 `/roles`；缺少该权限时退化为精确 `role_id` 输入框。
- `/users?role_id=` 深链直接生效：具备 `roles.read` 时显示角色名 chip，否则显示原始 id；无效或已软删除角色显示空态与清除筛选入口，不崩溃。角色删除 409 的“查看使用该角色的用户”跳转始终可用；该链接仅在同时具备 `users.read` 时显示。
- API 的 status 与 role_id 都是单值，不做多选伪装。

**创建用户**

- 字段：username、email、display_name（可选）、phone（可选）、密码模式（默认自动生成 / 手动设置 + 确认）、角色（可选）。
- 角色区要求同时具备 `users.assign_role` 与 `roles.read`；基线 `user` 以锁定勾选展示且始终保留，只提交额外选中的角色 id，服务端自动补基线。缺少任一权限时隐藏角色区并且不发送 `role_ids`；没有 `users.assign_role` 也能创建仅含基线角色的用户。
- 自动生成由服务端返回 24 字符临时密码；手动设置必须通过密码策略（15–128、NFC、blocklist、不截断），客户端只校验长度与确认一致，blocklist 与保留名交给服务端 422。
- 客户端 Zod schema 做成接受 `t` 的工厂函数，在组件内按 locale 记忆化；提交前 trim/小写 username，email 做 trim + NFC + 小写，服务端仍权威。
- 提交用 `runWithReauth` 包装（`POST /users` 一律要求 step-up）；pending 时禁用提交。
- 成功后关闭创建对话框并展示一次性临时密码面板；关闭面板后停留在列表、失效 `['users']`、toast“用户已创建”。当前筛选可能隐藏新用户时不自动修改筛选。
- 422 `validation_failed` 按 `errors[].pointer` 映射到字段；409 `identifier_taken` 有 pointer 落字段，否则表单级提示；403 `permission_subset_violation` / 422 `unknown_role` 在角色区内联展示。

**编辑用户**

- 字段：display_name、username、email、phone；用行内 `UserSummary` 预填，不额外 GET。
- 只有 dirty 字段才进入 PATCH；绝不发送空对象（避免 422 `empty_update`）；phone 清空时发送显式 `null`；display_name 为空视为校验错误，不允许清空；username/email 必填。
- 提交前 trim/小写 username，email 做 trim + NFC + 小写；服务端仍权威。
- 仅当 username 或 email 发生变化时才需要 step-up，用 `runWithReauth` 包装 PATCH；表单内提示“修改登录标识会改变下次登录方式并写入审计”。display_name/phone 修改不包装。
- 关闭 dirty 表单需确认；保存出错保留表单与输入。
- 409 `identifier_taken` 若响应带 `errors[].pointer` 就落到对应字段，否则对话框内表单级错误；403 `out_of_scope` / `self_action_forbidden` 在对话框内联展示。
- 成功后关闭对话框、失效 `['users']` 列表与详情、toast。
- 管理员不能通过用户管理编辑自己；编辑入口对自己禁用，tooltip 指向 Settings/Profile。

**停用、启用与删除策略**

- v1 不显示任何删除入口；停用是唯一“移除使用权”的动作。
- 停用需确认对话框：标题“停用用户 {{username}}？”；说明“停用后该用户会立即退出所有设备并无法登录；资料、角色与审计记录会保留，可随时重新启用。”；按钮“取消 / 停用”；不做输入用户名的强确认。
- 启用不弹确认，直接执行。
- 停用与启用都属于 `PUT /users/{id}/status`，都要求 step-up，因此都用 `runWithReauth`。
- 最后一位超级管理员由服务端返回 409 `last_super_admin`；在对话框内展示并保持打开，不预先猜测。
- 自我停用/启用禁用并给 tooltip。
- 已停用用户仍可编辑、重置密码、分配角色、撤销会话；解锁仅在 `locked_until > now()` 时显示。
- 列表 status 筛选只提供 active/disabled；锁定与待改密用独立标记。

**重置密码与一次性临时密码**

- 重置对话框提供“自动生成（默认）/ 手动设置”，并说明后果：目标用户全部会话立即撤销、下次登录必须改密、临时密码 72 小时有效。
- 对已停用用户也可重置；自我重置禁用。
- 提交用 `runWithReauth` 包装。
- 成功后对话框切换为一次性密码面板：等宽显示密码、复制按钮、本地时区到期时间；明确“关闭后无法再次查看”；关闭需点“我已安全保存”或二次确认；手动设置的密码同样展示响应中的值；关闭时清掉内存引用。
- 密码值不进入 query cache、localStorage/sessionStorage、日志或分析事件；v1 不允许管理员设置永久密码。
- 创建与重置共用同一个一次性密码展示组件。

**角色分配**

- 入口在行操作与详情 Sheet，需要同时具备 `users.assign_role` 与 `roles.read`；缺少任一权限时隐藏入口，API 契约不变。
- 创建表单与角色分配对话框共用 `RoleMultiSelect`：
  - 一次拉取全部活跃角色（`GET /roles`，`page_size=200`，若 `total > 200` 继续翻页），缓存为角色选项 query。
  - 始终显示客户端搜索框，按角色名称过滤；对话框内不分页。
  - 基线 `user` 固定勾选且禁用；其他角色按 `assignable` 展示，但不隐藏不可授予项：内置 `admin`/`super_admin` 在非超管时禁用并提示“仅超级管理员可授予”；自定义角色超出有效权限时禁用并提示“超出你的有效权限”。
  - 每个角色显示权限数量；v1 不展开权限名、不额外请求 `/permissions`。
  - 显示“已选 n 项”。
- 当前已分配角色预选；保存提交完整角色集合，服务端自动保留基线 `user`。
- 添加或移除 `admin`/`super_admin` 时对话框显示警告横幅，但不额外二次确认。
- 提交用 `runWithReauth`；403 `permission_subset_violation`、422 `unknown_role`、409 `last_super_admin` 在对话框内展示。
- 成功后失效 `['users']` 列表与详情、toast；角色选项缓存不失效。
- 自我分配禁用。
- 角色管理页保持只读，不提供成员分配/移除；`/users?role_id=` 由本页承接。

**解锁、撤销会话与批量边界**

- 解锁入口仅在 `locked_until > now()` 时显示；不要求 step-up；成功 toast；幂等；自我解锁禁用。
- 撤销全部会话在拥有 `sessions.revoke` 时显示；确认文案“撤销 {{username}} 的全部会话？该用户会立即退出所有设备并需要重新登录。”；提交用 `runWithReauth`；自我撤销禁用。
- 查看详情始终可用（`users.read`）；行点击打开 Sheet，操作菜单阻止冒泡。
- 所有针对自己的管理动作（编辑、启停、重置、角色、撤销会话、解锁）一律禁用，tooltip“不能在用户管理中操作自己；请在个人设置中修改”。
- `out_of_scope` 目标无法从列表预判：列表已按管理范围过滤；深链或过期数据返回 403 时在 Sheet/对话框内展示错误并失效用户查询，不做猜测性禁用。
- v1 不做批量操作、批量选择列、批量工具栏、批量删除或批量启停；不做乐观更新。

**缓存与状态**

- Query key：`['users','list',params]`、`['users','detail',id]`、`['roles','options']`。
- 所有管理 mutation 不做乐观更新；统一 pending 禁用 + 成功后失效/重新拉取。
- 列表首载用骨架；切换筛选/翻页保留旧数据并显示轻量 loading；请求失败在表格上方显示可重试错误，不静默清空。
- 空态区分“还没有用户”与“没有匹配结果”（后者提供清除筛选）。
- 页码越界自动回到最后一页。
- 不轮询；提供手动刷新；继承全局 `refetchOnWindowFocus`、401/403/429 不重试与只读 5xx 最多重试一次。
- 失效规则：创建/编辑/状态/角色成功后失效 `['users']`；重置/撤销/解锁失效列表与详情；`['auth','me']` 不受影响（自我操作已禁止）。
- 所有失败保持对话框打开并保留表单值；step-up 取消同样保留；临时密码不进入任何 query cache。

**错误映射**

- 422 `validation_failed` 的 `errors[].pointer` 映射到 `/username`、`/email`、`/display_name`、`/phone`、`/password` 字段；`/role_ids` 或 `/role_ids/N` 归入角色区；未知 pointer 归入表单级。
- `errors[].code` 映射 `validation.<code>`，`params` 做命名插值，未知 code 回退 `validation.invalid`；有字段错误时不再重复顶层 `validation_failed`。
- 409 `identifier_taken`：有 pointer 则字段级，否则对话框内表单级提示。
- 403 `out_of_scope` / `self_action_forbidden` / `permission_subset_violation` 在对话框或 Sheet 内展示；行级动作用 toast；`out_of_scope` 同时失效用户查询。
- 409 `last_super_admin` 在状态/角色对话框内展示。
- 429 读取 `Retry-After` 倒计时并禁用提交。
- 5xx 与网络错误展示可复制的 `request_id` 与重试入口；未知错误码回退 `errors.common.unexpected`。

### 9.9 Roles 页面

**信息架构**

- 独立路由：`/roles`（列表）、`/roles/new`（创建）、`/roles/$roleId`（详情/编辑）。列表行/名称进入详情；创建与编辑不使用抽屉或对话框。
- 内置角色有只读详情页；自定义角色在可管理时显示编辑表单。
- 详情页提供面包屑与返回列表；保存入口放在页头右侧与页面底部各一个，二者等价。

**保存模型**

- 单次保存，只提交 dirty 的部分：元数据 → `PATCH /roles/{id}`；权限 → `PUT /roles/{id}/permissions`；两者都变时先 PATCH 后 PUT。
- 不新增合并端点，不采用“基本信息/权限各自保存”的分段模型。
- 部分失败：PATCH 成功、PUT 失败时保留权限 dirty 状态，提示“基本信息已保存；权限未保存”，重新拉取角色；表单保留修改，可直接重试。
- 创建：`POST /roles` 一次提交 `{name, description?, permission_keys}`；至少一项权限；名称 1–64、活跃角色中不区分大小写唯一、内置名保留。
- 内置角色与 `manageable=false` 的角色只读，不渲染保存入口。

**权限矩阵**

- 完整权限目录按资源分组；组名与权限名使用后端 `display_name_zh` / `display_name_en`，按当前 locale 选择；顺序以代码目录为准。
- 非可授予项**完整显示但禁用**，标记“超出你的有效权限”并给出 tooltip；不在生产 UI 使用“隐藏”模式。
- 每个资源组提供三态“全选本组”，只作用于可授予项；卡片头部显示“已选 n 项”。v1 不提供全局全选。
- `super_admin` 内置角色：完整矩阵全选 + 禁用，横幅说明“自动拥有全部权限（含未来新增）”。
- 内置 `admin` / `user`：显示各自固定权限集的只读矩阵；`user` 显示“不授予管理权限”空态与基线角色说明。
- 自定义但 `manageable=false` 的角色：只读矩阵 + 横幅说明 `permission_subset_violation`。
- 提交校验：至少一项权限（422 `empty_permissions`）；写入内置角色 403 `role_immutable`。

**列表**

- 列：名称（内置徽章 + 本地化名称 + code chip）、描述、权限数、类型/只读徽章、更新时间、操作。
- **不显示分配用户数**；不为一个 UI 提示新增 API 字段；删除冲突依赖 409 `role_in_use`。
- 筛选：`q` 对名称/描述/内置标识做大小写不敏感子串搜索（输入防抖 300ms）；`is_builtin` 三选一（全部/内置/自定义）。v1 不做“仅可管理”筛选，`manageable=false` 用只读徽章表达。
- 排序：默认 `name asc`；白名单 `name` / `created_at` / `updated_at`；服务端排序。
- 分页：offset 分页，默认 20，选项 10/20/50/100，`page_size ≤ 200`；页码、页大小、筛选与排序同步到 URL。
- 内置角色不置顶；用类型筛选与徽章区分。
- 列表提供空态（清除筛选）、加载骨架、错误重试；操作按钮：查看始终可用；编辑/删除按目标资格门控。

**删除与 Users 页职责**

- 删除仅对自定义且 `manageable=true` 的角色开放，并需要 `roles.delete`。
- 确认框展示角色名称、权限数、软删除不可恢复警告；不做手输角色名的强确认。
- 409 `role_in_use`：在删除对话框内展示错误码与说明，并提供“查看使用该角色的用户”按钮，跳转 `/users?role_id={id}`；Users 页按 ADR-0004 支持 `role_id` 筛选。
- 角色详情页提供只读的“查看使用该角色的用户”链接，不提供分配/移除成员的操作。
- 角色分配入口唯一放在 Users 页；角色管理页不重复实现最后超管、自我降级、自我停用保护。

**step-up 与未保存拦截**

- 角色所有变更（`POST /roles`、`PATCH /roles/{id}`、`PUT /roles/{id}/permissions`、`DELETE /roles/{id}`）都要求 step-up；列表/详情/`GET /permissions` 不要求。
- 被动触发：保存/删除返回 403 `reauth_required` 后弹出重新认证对话框；成功则自动重试原操作一次；取消则保留表单、不执行操作；密码错误 422 `invalid_current_password` 在对话框内提示。
- 单次保存的两个请求共享一次 step-up；第二个请求落在 300 秒窗口内直接通过。
- 站内拦截：返回列表、关闭编辑、侧栏导航时若表单 dirty，弹出确认框（继续编辑 / 放弃更改）。
- 硬导航拦截：dirty 时注册 `beforeunload`，由浏览器原生提示，不自定义文案。
- 保存/删除成功后清除 dirty 与拦截。

**错误与 i18n**

- `/roles` 路由要求 `roles.read`；侧栏菜单无权限时隐藏；直接访问无权限显示 403 页面，不显示空列表，也不静默重定向。
- 404：角色不存在或已软删除；409：`role_in_use`、`role_name_taken`；422：`validation_failed` / `empty_permissions` / `invalid_permission`；403：`role_immutable`、`permission_subset_violation`、`reauth_required`。
- 错误文案按稳定 `code` 映射，角色相关 key 归入 `errors.role.*`；无权限 → 不渲染；目标级不可管理 → 禁用 + tooltip；内置角色 → 只读 + 横幅。
- 内置角色名称使用 i18n key（如 `roles.builtin.super_admin`），旁边显示 `super_admin` 等 code chip；自定义角色名称按用户输入展示；权限组/权限项使用后端双语显示名，缺失时回退 key。

### 9.10 审计日志页面

**信息架构与权限**

- 列表路由 `/audit-logs`；详情为 URL 同步的右侧 Sheet（`/audit-logs?detail=<id>`），关闭时清除 `detail`。深链只展示详情，不自动改列表筛选；详情不存在或已过保留期时在 Sheet 内显示空态。
- 每次打开详情都重新 GET，不缓存、不预取、不 hover 预加载；确保每次查看都写 `audit_log_accessed`。
- 侧栏在 Roles 之后新增“审计日志”，按 `audit.read` 显示；无权限直接访问显示 403 页面。列表与详情同一权限，不新增独立详情权限，读取审计不要求 step-up。

**列表**

- 默认列：时间、级别、事件、操作者、对象、结果、IP；`request_id` 默认隐藏（列切换可开）；`user_agent` 仅在详情。
- 事件列显示本地化 label + 原始 `action` code；未知 action 只显示 code。操作者显示 `actor.label`；有 `actor.id` 且具备 `users.read` 时链接用户详情，`anonymous`/`cli`/`system` 用 i18n 标签。对象显示本地化 `target_type` + `target.id`（截断可复制）或 `target.label`。
- 排序仅 `occurred_at`（默认 desc，可 asc）；分页默认 20，选项 20/50/100，上限 200，显示 `total`；筛选、排序、页码、页大小全部同步 URL；筛选变化重置页码并关闭详情 Sheet。
- 不自动轮询；提供手动刷新（刷新不改变当前绝对时间窗）。行点击/Enter 打开详情；行内 actor/target 链接阻止冒泡并跳转对象详情。
- 加载骨架、空态（清除筛选/建议扩大时间范围）、错误重试；列表请求失败不静默清空。

**筛选**

- 布局：主筛选行 = 时间范围、操作者、事件类型、级别、结果、刷新；“更多筛选” popover = 目标类型/目标 ID、`request_id`、IP；已启用筛选显示为可单独移除的 chips，并提供“清除全部”。
- 选择类即时生效；文本类 300ms 防抖；自定义时间范围点“应用”。
- 操作者：用户选择器（`GET /users?q=`，需 `users.read`）+ 精确 `actor_id` 输入兜底；不改 API。用户选择器受管理范围过滤，范围外用户可从日志复制 `actor.id` 再筛。
- `action` / `severity` / `result` 均为单选；事件类型按认证/授权/用户/角色/系统/审计分组下拉。
- IP：精确 `ip` 过滤（不做 CIDR）；服务端规范化 IPv4/IPv6。
- 时间：预设最近 15 分钟/1 小时/24 小时/7 天/30 天/自定义；默认最近 30 天。选中预设时解析为绝对 `from`/`to`（UTC）写入 URL；分页、刷新、分享都使用同一绝对窗口，避免 offset 分页重复/漏行；提供“重新计算为最近 N”。最大跨度 90 天；前端校验 `from < to` 与跨度，服务端 422 `invalid_time_range` / `time_range_too_large` 兜底。
- 显示按浏览器本地时区，列头/详情标注时区（如 `UTC+08:00`），tooltip 与复制提供 RFC3339 UTC；筛选输入按本地时区解释，提交前转 UTC。

**详情**

- 概要：级别、事件（本地化 label + code）、结果、时间、操作者、目标、IP、User-Agent、`request_id`（可复制）、审计日志 id。
- `details` 提供“结构化”与“原始 JSON”双视图。结构化用通用递归渲染 + 已知字段的本地化标签/格式化（时间、UUID、计数、权限 key chips、角色 id 链接）；未知字段原样显示；长数组默认折叠。原始 JSON pretty-print、可复制（复制脱敏后的内容）。
- 脱敏：后端已保证密码/令牌/哈希等不进入审计；前端再做精确 key 黑名单兜底（`password`、`password_hash`、`temporary_password`、`access_token`、`refresh_token`、`authorization`、`cookie`、`secret`、`api_key`、`private_key`、`client_secret`），命中值显示 `***`，不提供“显示原值”。不使用宽泛的 `key` 子串匹配，避免误伤 `permission_keys`、`key_type`、`old_kid` 等诊断字段。
- 操作：复制日志 id、`request_id`、脱敏 JSON；“筛选该操作者的其他事件”（设 `actor_id`）、“筛选该对象的其他事件”（设 `target_type`/`target_id`）、“筛选同一 `request_id`”；具备权限时提供操作者/目标对象的详情页链接。不做导出、删除、编辑；v1 不做 Sheet 内上一条/下一条。
- 页头一行提示“查看详情会写入审计访问记录”。

**保留期与边界**

- 页头“保留策略” popover 说明 info/warning 至少 180 天、error/critical 至少 365 天、单次查询最多 90 天、清理由运维任务执行；查询范围可能超出保留期时提示更早记录可能已清理。不显示每条到期时间。
- 不做归档、删除、批量导出/CSV；不暴露 `chain_seq`/`row_hash`，不提供链校验按钮；链校验由后端/运维工具承担。
- 前端静态 action 目录（action → zh/en 标签、分类），未知 action/`target_type`/`severity`/`result` 回退原始 code。

### 9.11 Settings 子页

- **Profile**：只编辑 `display_name`（`PATCH /auth/me`）；username、email、phone、status、roles 只读，并标注 username/email/phone 由管理员维护。移除上游 bio、URLs 与硬编码 email 选项。
- **Account**：改密表单（当前密码、新密码、确认）调 `POST /auth/change-password`；成功后更新 access token、失效并重新拉取 `['auth','me']`，提示其他设备已退出；若原本是受限会话，回到登录前目标。提供“退出所有设备”按钮调 `POST /auth/logout-all`。语言选择保留在 Account，但只写本地 `locale` Cookie（`zh-CN`/`en-US`），不接后端。
- **Appearance**：保留主题/字体本地偏好；v1 仅 LTR，不再提供用户可见的方向切换；`DirectionProvider` 与逻辑 CSS 保留，`dir` 固定为 `ltr`；不接后端。
- **Notifications / Display**：保留页面，但只做本地偏好（localStorage），明确标注 v1 不接后端、不发送通知；Display 的侧栏偏好可应用到侧栏，不新增 API。移除上游 `showSubmittedData` 作为生产提交路径。
- 强制改密使用独立 `/change-password` 路由，不复用 Settings 的修改流程；改密成功后的跳转由 §9.5 的路由守卫处理。

### 9.12 i18n、日期与 RTL

**选型与资源**

- `i18next` + `react-i18next`；两种语言全部静态打包，不使用 `i18next-http-backend`、不按语言懒加载。
- 目录：
  - `src/i18n/index.ts`：初始化、`SUPPORTED_LOCALES`、`resolveInitialLocale()`、`setLocale()`、`i18n` 导出。
  - `src/i18n/locales/zh-CN/<namespace>.json`、`src/i18n/locales/en-US/<namespace>.json`。
  - `src/i18n/types.d.ts`：从 zh-CN 资源生成 `CustomTypeOptions`，提供严格 key 类型。
- namespace：`common`、`nav`、`auth`、`users`、`roles`、`audit`、`settings`、`errors`、`validation`。
- 初始化：`lng` 由 `resolveInitialLocale()` 决定；`fallbackLng: 'en-US'`；`supportedLngs: ['zh-CN', 'en-US']`；`load: 'currentOnly'`；`defaultNS: 'common'`；`ns` 显式列出全部 namespace；`initImmediate: false`；在 `main.tsx` 渲染前 import `src/i18n`；`interpolation: { escapeValue: false }`；`returnNull: false`。
- 开发/测试环境 `missingKeyHandler` 报错；生产环境回退到 `en-US`，再缺失则显示 key，不静默吞掉。
- 复数使用 i18next 的 `Intl.PluralRules` 后缀（如 `_one`/`_other`），不手写单复数；中文只有 `_other`，英文需要 `_one`/`_other` 的键由 en-US 资源提供。
- 禁止运行时拼接 key（包括 `errors.${code}` 这类模板字符串）；错误码、审计动作等动态 code 必须经过静态映射表，未知 code 走显式 fallback。
- 资源格式用嵌套 JSON；键路径用 `.` 分隔，namespace 用 `:` 分隔。

**语言解析、持久化与切换**

- 初始语言解析顺序：
  1. 本地 `locale` Cookie，值必须是受支持 locale；非法/缺失则忽略。
  2. `navigator.languages` 中第一个 `zh*` → `zh-CN`，第一个 `en*` → `en-US`。
  3. 默认 `zh-CN`。
- 偏好存储：Cookie `locale`，`Path=/; Max-Age=31536000; SameSite=Lax`，不设 `Domain`、不设 `HttpOnly`、不设 `Secure`（非敏感偏好，且本地 HTTP 开发需要）；不写入后端用户资料。
- 切换语言：`i18n.changeLanguage` 立即生效，不刷新页面；同步更新 Cookie、`<html lang>`、`document.documentElement.dir`（固定 `ltr`）与文档标题。语言切换不改变路由、不重置表单，也不重新请求 API。
- Account 语言选项只保留“简体中文 / English”两项，用各自语言显示；不显示上游未支持语言。
- 文档标题：根路由挂载 `DocumentTitle`，从 `useMatches()` 取最深的 `staticData.titleKey`，用 `t()` 生成 `${title} · ${t('common.appName')}`；语言变化时通过 `useTranslation()` 重新渲染并更新 `document.title`；`index.html` 只保留中性 fallback 标题 `rs-admin`，移除上游英文 description/OG/Twitter 文案；不引入 `react-helmet-async`。

**key 命名与错误文案边界**

- UI 文案 key 使用 lowerCamelCase 的语义路径，例如 `nav.users`、`auth.signIn.title`、`settings.account.language`、`roles.builtin.super_admin`、`common.actions.save`。
- 后端稳定 code 的 key 保留 snake_case 原文，例如 `errors.auth.invalid_credentials`、`errors.role.role_name_taken`、`validation.invalid_format`、`audit.actions.authn_login_success`。
- 顶层错误统一 `errors.<domain>.<code>`，domain 为 `auth`、`user`、`role`、`audit`、`common`；字段错误统一 `validation.<code>`。共享 code 优先 `errors.<domain>.<code>`，缺失时回退 `errors.common.<code>`，再回退 `errors.common.unexpected`。
- 内置角色名称用 `roles.builtin.<code>`（`super_admin`/`admin`/`user`），旁边显示稳定 code chip；自定义角色名称按用户输入原样展示，不翻译。
- 权限组/权限项使用后端 `display_name_zh` / `display_name_en`，按当前 locale 选择；缺失或为空时回退原始 permission key，不在前端另建权限名称表。
- 审计日志的 `action`、`target_type`、`severity`、`result` 由前端静态目录本地化：`audit.actions.<action>`、`audit.targetTypes.<type>`、`audit.severity.<value>`、`audit.results.<value>`；未知 action/target_type 显示原始 code，未知 severity/result 同样回退原始值。`actor_label` 为快照，`anonymous`/`cli`/`system` 用 `audit.actors.*`，其他显示原值。
- 插值只用命名参数，如 `{{count}}`、`{{seconds}}`、`{{requestId}}`、`{{max}}`；参数名 lowerCamelCase，值由 React 转义，不使用 HTML 片段。
- 复数 key 使用 i18next 后缀；禁止把 `count` 拼进句子后手工处理单复数。
- key 只能静态引用；错误码/动作目录用 `Record<Code, TranslationKey>` 映射，未知 code 走 fallback；不得由后端字符串直接拼出 key。

**错误码映射**

- 顶层错误：`translateApiError()` 读取 `ApiError.code`，经静态映射选择 `errors.<domain>.<code>` → `errors.common.<code>` → `errors.common.unexpected`；`title`/`detail` 不作为主要文案，仅在开发 console 中保留诊断。
- 字段错误：`errors[].pointer` 映射到 React Hook Form 字段名；`errors[].code` 映射到 `validation.<code>`，`params` 作为命名插值；未知 pointer 归入表单级错误，未知字段 code 回退 `validation.invalid`；若存在字段错误，不再重复展示 `validation_failed` 顶层文案。
- `request_id`：从 ProblemDetails 或 `X-Request-Id` 读取；5xx/网络错误、认证/权限错误、无字段映射的页面级错误展示可复制 `request_id`；纯字段校验不额外显示；技术细节行保留原始 `code`，不展示原始 `detail`。
- `Retry-After`：429 读取响应头（秒数），Clamp 为非负整数；缺失/非法回退 60 秒；展示倒计时并禁用提交，归零后恢复；不做自动重试。账号锁定与 IP/全局限流分别使用 `errors.auth.login_locked` / `errors.auth.rate_limited`（登录/重新认证上下文）或 `errors.common.rate_limited`（其他上下文）。
- `params`：只做文本插值，不注入 HTML；`password_policy_violation` 的 `params.reason` 用静态映射选择 `validation.password.<reason>`，未知 reason 用通用文案。
- 认证错误映射至少覆盖：`invalid_credentials`、`account_disabled`、`temporary_password_expired`、`login_locked`、`rate_limited`、`session_expired`、`refresh_token_invalid`、`refresh_reuse_detected`、`reauth_required`、`invalid_current_password`、`password_change_required`、`password_policy_violation`、`password_unchanged`、`csrf_failed`。
- 角色错误映射至少覆盖：`role_immutable`、`permission_subset_violation`、`role_name_taken`、`role_in_use`、`empty_permissions`、`invalid_permission`、`unknown_role`；用户错误映射至少覆盖：`identifier_taken`、`self_action_forbidden`、`out_of_scope`、`last_super_admin`；审计/通用错误覆盖：`invalid_time_range`、`time_range_too_large`、`validation_failed`、`invalid_json`、`unknown_field`、`unsupported_media_type`、`method_not_allowed`、`payload_too_large`、`invalid_sort`、`page_size_too_large`、`empty_update`、`forbidden`、`not_found`、`internal_error`、`service_unavailable`。
- 前端 Zod 校验消息：schema 做成接受 `t` 的工厂函数（如 `makeSignInSchema(t)`），在组件内按当前 locale 记忆化创建；消息使用 `validation.*`，不硬编码英文。语言切换后 schema 随 `t` 重建，已显示的错误在下次校验/输入时更新。
- 错误码目录以 §7.3 为准；新增未知 code 必须回退，不得导致页面崩溃或显示 `title`/`detail`。

**日期、数字与时区**

- 展示层统一 `Intl.DateTimeFormat` / `Intl.NumberFormat`，按当前 locale；`date-fns` 只保留 calendar/date-picker 的 locale 支持。
- API 时间一律 RFC3339 UTC；展示转浏览器本地时区。审计等绝对时间场景标注时区（如 `UTC+08:00`），tooltip 与复制提供 RFC3339 UTC；筛选输入按本地时区解释、提交前转 UTC。
- 格式化 helper 集中在 `src/lib/format.ts`，不引入 moment/dayjs。

**RTL**

- v1 界面仅 LTR；`zh-CN` 与 `en-US` 都映射为 `ltr`。
- 保留 `DirectionProvider`、Radix `DirectionProvider` 与逻辑 CSS 属性（`ps-*`、`pe-*`、`ms-*`、`me-*`、`text-start/end` 等）；保留上游对 shadcn 组件的 RTL 修改。
- 移除 ConfigDrawer/Appearance 中的手动 LTR/RTL 切换；`dir` Cookie 不再作为用户可写偏好；`document.documentElement.dir` 固定为 `ltr`。

### 9.13 bun 迁移、CI 与许可

- `package.json`：`"packageManager": "bun@1.4.2"`；`"engines": { "node": ">=22.18" }`（orval 8.46 下限；Bun 兼容性需在实现首日验证）。
- 提交文本 `bun.lock`，删除 `pnpm-lock.yaml`；不提交 `node_modules`。
- 脚本：保留上游脚本名（`dev`、`build`、`lint`、`format`、`format:check`、`test`、`test:browser:install` 等），新增 `typecheck`（`tsc --noEmit`）、`check`（lint + format:check + typecheck + test + build 的聚合）与 `i18n:check`（key 对齐检查）。文档/CI 统一 `bun run <script>`、`bunx`，不再出现 `pnpm`/`pnpm dlx`。
- CI（前端部分）：`oven-sh/setup-bun` 固定 bun 1.4.2，缓存 Bun 安装缓存；`bun install --frozen-lockfile`；依次运行 lint、format check、typecheck、i18n key 检查、browser install、test、build；orval 生成客户端后 `git diff --exit-code` 防漂移；Playwright 使用 `bunx playwright install --with-deps chromium`。
- Docker（前端构建阶段）：构建阶段 `oven/bun:1.4.2`，`bun install --frozen-lockfile` + `bun run build`；`VITE_API_BASE_URL` 作为 build arg 注入；产物交给静态运行时（nginx），生产镜像不含 Node/bun；`.dockerignore` 排除 `node_modules`、`dist`、`.git` 与本地缓存。
- 许可：保留上游 `LICENSE` 全文；新增 `THIRD-PARTY-NOTICES.md`（至少列出 shadcn-admin、shadcn/ui、Radix UI、Tailwind CSS、Lucide 等直接依赖的许可证与版权行）；README 增加归属说明；删除 Clerk 品牌素材；替换品牌名称/logo 时避免暗示官方关联。

### 9.14 前端测试与验收

- 单元测试：`hasPermission`、redirect 净化、auth store、401 单飞刷新、`runWithReauth` 单飞/取消/只重试一次、locale 解析、错误映射。
- 集成测试（Vitest browser mode + MSW）：登录成功与错误矩阵、冷启动 refresh 成功/401/网络失败、401 刷新后重试与刷新失败登出、未登录/无权限/受限会话三类守卫、侧栏与按钮门控、Profile 保存、改密、重新认证对话框。
- i18n 测试初始化固定 `zh-CN`（独立 i18n 实例并在每个测试后重置）；少量语言切换、错误映射用例显式跑 `en-US`。
- CI 增加 key 对齐检查：递归比较 zh-CN/en-US 各 namespace 的 key 集合，缺失/多余都失败；开发/测试 `missingKeyHandler` 直接抛错。
- 文本易变的断言改用 `data-testid`/ARIA role；只有文案本身是验收入口时才断言具体字符串。
- 至少覆盖：语言解析与 Cookie 持久化、切换后 `<html lang>`/标题/文案更新、错误码映射（认证/角色/用户/通用）、未知 code 回退、`Retry-After` 倒计时、字段错误 pointer 映射、内置角色名称与 code chip、权限显示名回退、审计 action/target_type 回退。
- 验收点见 §14；前端不得引入 E2E 之外的重复后端集成测试；真实后端联调与部署验证由 §12.5 的 E2E/发布 smoke 承担。

---

## 10. 部署、可观测与 CI

### 10.1 部署拓扑与主机基线

- 单台 Linux 主机 + Docker Compose，项目名 `rs-admin`。服务：
  - `gateway`：nginx，终止 TLS、按 Host 路由、边缘限流与 HSTS；只发布 `80/443`。
  - `web`：多阶段构建的静态前端运行时（`oven/bun:1.4.2` 构建 → `nginx:1.29.8-alpine` 运行），只在 Compose 内网可达。
  - `api`：Rust/Axum 镜像，只在 Compose 内网监听 `8080`（业务）与 `9100`（metrics）。
  - `db`：`postgres:18.6-alpine`，Compose profile `local-db`，只在 Compose 内网可达，不发布主机端口。
  - `maintenance` / `backup`：一次性任务容器，profile `tools`，由主机 systemd timer 触发。
  - `observability`：独立 Compose 文件，包含 Prometheus、Alertmanager、Grafana、Loki、Promtail、postgres_exporter、node_exporter、alert webhook 适配器。
- 数据库可分离：默认 `COMPOSE_PROFILES=local-db` 启动本地 `db`；切换到外部 PostgreSQL 时移除该 profile，并在 `.env` 设置 `DATABASE_URL` 或 `DATABASE_URL_FILE`。迁移、健康检查、备份职责随 `DATABASE_URL` 指向切换；外部数据库的备份、小版本补丁与大版本升级由该实例的运维方负责，规格只记录最低保证（PG18、SCRAM、TLS `verify-full`、RPO/RTO）。
- 主机基线：4 vCPU / 8 GiB RAM / 100 GiB SSD；Ubuntu 24.04 LTS 或 Debian 12；Docker Engine 27+（Compose v2）；systemd + chrony；时区 UTC。低于 4 GiB RAM 或 2 vCPU 视为容量复查条件。
- 主机加固：SSH 仅密钥登录并限制来源；防火墙只放行 22（管理员来源）与 80/443；启用自动安全更新；Docker 日志驱动 `json-file` 限制 `max-size=10m`、`max-file=5`；`/etc/rs-admin` 与数据卷权限最小化；容器以非 root 运行，API/网关启用 `no-new-privileges`，API 根文件系统只读（`/tmp` 用 tmpfs）。
- 网络：固定 Docker bridge 子网（如 `172.28.0.0/16`），`TRUSTED_PROXY_CIDRS` 覆盖该子网或网关容器地址；`api`/`db`/`web` 不发布主机端口；Prometheus 通过 Compose 内网抓取 `api:9100`。
- 首发单副本：API 1 个副本；数据库单实例（本地）或外部单实例。多实例就绪是设计约束，不是首发验收项；扩到 2+ 副本的 checklist 见 §10.11。

### 10.2 域名、网关、TLS 与静态托管

- 域名占位：`admin.example.com`（前端）与 `api.example.com`（API），两者同属同一 registrable domain，保持 ADR-0001 的同站子域假设。上线前必须完成「域名/DNS/证书/网关归属」检查清单：
  - DNS A/AAAA 指向部署主机；`80/443` 对 ACME 与用户可达；
  - 前端与 API 仍同站；若无法同站，按 ADR-0001 回退 BFF，不把 `SameSite=None` 第三方 Cookie 作为生产主方案；
  - 更新 `ADMIN_ORIGIN`、`CORS_ALLOWED_ORIGINS`、`PUBLIC_API_ORIGIN`、CSP `connect-src`、`VITE_API_BASE_URL` 与 HSTS `preload` 决策；
  - 记录证书签发方式、到期时间与续期责任人。
- 网关选型 nginx：`limit_req` 原生满足 `/auth/*` 每 IP 60 次/分钟粗粒度限流；Caddy 需要第三方插件才能内置限流，v1 不选。
- TLS：TLS 1.2/1.3，现代密码套件；HTTP 一律 301 到 HTTPS；HSTS `max-age=31536000; includeSubDomains`，`preload` 待实际域名确认后追加；证书用 Let's Encrypt + certbot（ACME HTTP-01，webroot 挂载到网关），systemd timer 自动续期，续期后 `nginx -s reload`；证书到期告警见 §10.7。
- 路由与代理：
  - `admin.example.com` → `web:80`；`api.example.com` → `api:8080`。
  - 网关覆盖而非追加 `X-Forwarded-For`（`proxy_set_header X-Forwarded-For $remote_addr;`），设置 `X-Forwarded-Proto`、`Host`；忽略客户端提供的 `X-Real-IP`/`Forwarded`；应用侧只信任 `TRUSTED_PROXY_CIDRS` 内的 peer。
  - 代理超时 30 秒（connect/read/send），与 API 请求超时一致；认证端点 `client_max_body_size 1k`，其他路径 `64k`。
  - `/auth/*` 使用 `limit_req_zone $binary_remote_addr zone=auth:10m rate=60r/m;` + `burst=10 nodelay`，超限返回 429 并带 `Retry-After`；应用层账号/IP/全局限流仍是权威。
- 静态前端：
  - 构建阶段 `oven/bun:1.4.2`，`bun install --frozen-lockfile` + `bun run build`；`VITE_API_BASE_URL` 作为 build arg 注入（生产占位 `https://api.example.com/api/v1`）。
  - 运行阶段 `nginx:1.29.8-alpine`，不含 Node/bun；SPA fallback `try_files $uri /index.html`。
  - 缓存：带 hash 的静态资源 `Cache-Control: public, max-age=31536000, immutable`；`index.html` 与错误页 `no-cache`；启用 gzip/brotli。
  - CSP 在 `web` 静态服务配置，按 §8.6 原文；不得放宽 `script-src` 的内联限制。CSP 由网关或 `web` 统一输出，二选一但只能有一处，避免重复头。
- 生产不暴露：`/metrics`、`/health/*`、`/api-docs/*` 与 Swagger UI 不通过公网网关暴露；`/metrics` 仅在 Compose 内网可达，`/api-docs` 生产关闭。

### 10.3 配置、密钥与数据库分离

- 非敏感配置：`.env`（从 `.env.example` 复制，权限 `0640`），包含端口、域名、连接池、超时、限流阈值、保留期、镜像 tag 等；不得包含密钥。
- 密钥交付：主机目录 `/etc/rs-admin/secrets/`（`0400`，root 所有）；Compose 顶层 `secrets:` 从该目录取文件，挂载到容器 `/run/secrets/<name>`。应用优先读取 `*_FILE` 环境变量，环境变量明文作为回退（仅开发/测试）。
- 必需 secret 文件：按 §8.7 的 JWT/refresh 派生/throttle/审计链密钥与数据库连接；新增：
  - `DATABASE_URL_FILE`（外部数据库或本地数据库 DSN，含 `sslmode=verify-full`）；
  - `INTERNAL_CA_CERT_FILE`（PostgreSQL 内部 CA）；
  - `ALERT_WEBHOOK_URL_FILE`（飞书/钉钉/通用 webhook）；
  - `PGBACKREST_REPO_CIPHER_PASS_FILE` 与 S3 兼容存储凭据（仅本地数据库 + backup profile）。
- 启动校验：沿用 §8.7；额外校验 metrics 端口、`TRUSTED_PROXY_CIDRS`、`ALERT_WEBHOOK_URL` 可解析（为空允许但告警），以及当 `COMPOSE_PROFILES` 不含 `local-db` 时 `DATABASE_URL` 必须指向非本机地址。
- 轮换：按 §8.7；轮换后重启 `api`，写 `crypto_key_rotated` 审计。数据库密码轮换需要同时更新 secret 文件与 PG 角色密码，步骤写入部署手册。
- 数据库分离切换：改 `.env`（`DATABASE_URL[_FILE]`）+ 移除 `local-db` profile → 重建 `api`/`migrate`/`maintenance`；迁移仍由一次性 `migrate` 容器执行；本地 `db` 卷保留但不再挂载。

### 10.4 Axum 运行时配置

- 监听与端口：业务 `0.0.0.0:8080`；metrics `0.0.0.0:9100`；仅 Compose 内网可达。两个 listener 独立，metrics 端点不经过业务中间件。
- 连接池：`DATABASE_MAX_CONNECTIONS=10`/API 副本、`DATABASE_MIN_CONNECTIONS=1`、`acquire_timeout=3s`、`idle_timeout=10m`、`max_lifetime=30m`；PostgreSQL `max_connections=100`。容量公式：`副本数 × 10 + 迁移/维护/备份/监控连接 < 100`。
- 超时与请求限制：
  - 请求总超时 30 秒（`tower_http::timeout::TimeoutLayer`）；网关 30 秒；header 读取超时 10 秒；keep-alive 60 秒。
  - 请求体：认证端点 1 KiB，其他 JSON 端点 64 KiB；超限 413 `payload_too_large`。
  - 并发：`tower::limit::ConcurrencyLimitLayer` 保护下游（建议 100 in-flight）；Argon2 另有独立信号量。
- Argon2 并发：`PASSWORD_HASH_CONCURRENCY=max(1, min(4, CPU 核数 / p))`，默认上限 4；等待超过 2 秒返回 503 `service_unavailable` + `Retry-After`；容器内存限制 1 GiB（64 MiB × 4 + 运行时余量），CPU 限制 2。
- 优雅停机：监听 SIGTERM/SIGINT；停止接受新连接，等待 in-flight 最多 15 秒，随后 `pool.close().await`；Compose `stop_grace_period: 25s`。网关在滚动/重启时短暂 502 可接受。
- 启动顺序：加载并校验配置 → 连接数据库 → 同步权限目录与内置角色（advisory lock）→ 安装 metrics recorder 与 upkeep → 启动 listener。**不在启动时执行迁移**。
- 健康检查：
  - `GET /health/live`：进程存活即 200，不查数据库。
  - `GET /health/ready`：1–2 秒超时执行 `SELECT 1`，并校验 `_sqlx_migrations` 最新版本不低于内嵌 Migrator 的版本（数据库落后未就绪；数据库领先视为就绪，见 ADR-0008 2026-11-10 修订）；失败返回 503 `{"status":"unready"}`，不返回 SQL 错误或连接串。Docker healthcheck 与外部 uptime 使用该端点。
- `request_id`：服务端生成 UUIDv7，忽略客户端传入；写回 `X-Request-Id`；应用日志与审计共用该值作为 trace_id。
- CORS：`CORS_ALLOWED_ORIGINS=https://admin.example.com`；精确 Origin allowlist，不使用通配符/`null`；credentials 仅限 §7.11 的端点。

### 10.5 PostgreSQL 18、安全与备份

- 镜像固定：`postgres:18.6-alpine`（2026-11-07 核验的最新 18.x Alpine tag）；不使用浮动的 `18-alpine`。小版本升级 = 改 tag + 重启 + 验证；大版本升级见 §11。
- 数据与扩展：主机卷 `pgdata`；`shared_buffers=1GB`、`effective_cache_size=3GB`、`work_mem=8MB`、`maintenance_work_mem=256MB`、`max_connections=100`、`wal_level=replica`、`archive_mode=on`、`archive_timeout=60s`、`max_wal_senders=10`、`synchronous_commit=on`、`timezone=UTC`、`password_encryption=scram-sha-256`。启用 `pg_trgm`（`q` 子串搜索）与 `pg_stat_statements`；`uuidv7()` 为 PG18 内置。
- TLS：PostgreSQL 启用 `ssl=on`，服务端证书由内部 CA 签发，SAN 覆盖 `db`（本地）或外部数据库主机名；API 使用 `sslmode=verify-full` 并挂载 `INTERNAL_CA_CERT_FILE`；禁止 `sslmode=disable/prefer`。`pg_hba.conf` 使用 `hostssl ... scram-sha-256`，禁止 `md5` 与 PG18 `oauth` HBA（SQLx 不支持）。
- 数据库角色：
  - `rs_admin_migrator`：schema owner，拥有 DDL 权限，仅 `migrate` 一次性任务使用；
  - `rs_admin_runtime`：DML 权限，无 DDL；`audit_logs` 仅 `INSERT`/`SELECT`，`audit_chain_head` 有 `SELECT`/`UPDATE`；
  - `rs_admin_cleanup`：维护任务使用，拥有保留期清理所需的 `DELETE`（含 `audit_logs`），不用于应用运行；
  - 备份角色：按 pgBackRest 官方要求配置，限定来源地址与所需权限；若必须 superuser，`pg_hba.conf` 只允许备份容器地址并记录为例外。
- 备份：pgBackRest stanza `rs-admin`；每日全量 + 持续 WAL 归档到 S3 兼容对象存储（仓库加密，凭据在 secret 文件）；保留 30 天；备份完成后校验退出码并写 `pgbackrest_backup_last_success_timestamp` 指标。外部数据库不启用本地 pgBackRest，备份责任归外部运维方。
- 恢复：每季度从异地备份恢复到同机临时 PostgreSQL 容器/临时目录，验证 `SELECT 1`、迁移版本、关键表行数、审计链 HMAC 校验；记录 RTO 实测。RPO ≤ 15 分钟、RTO ≤ 2 小时。
- pgBackRest 运行方式：维护/备份镜像基于固定版本 PostgreSQL 镜像并安装固定版本 pgBackRest；`archive_command` 调用 `pgbackrest --stanza=rs-admin archive-push %p`；备份任务与 WAL 归档共享同一仓库配置与 secret。具体镜像版本在实现首日锁定并写入部署手册。

### 10.6 可观测性、日志与告警

- 应用日志：`tracing` + `tracing-subscriber` JSON 输出到 stdout；`EnvFilter::try_from_default_env()`，非法指令启动失败；字段白名单与脱敏沿用 §8.8。Docker `json-file` 仅作本地缓冲（10 MB × 5），Promtail 采集到 Loki，保留 14 天。
- 网关日志：nginx JSON access log 输出到 stdout；只记录 `$uri`（不含 query string）、方法、状态、耗时、上游耗时与 request_id（若存在）；不记录 `Authorization`、`Cookie`、`Set-Cookie`、请求体。
- PostgreSQL 日志：`log_min_duration_statement=500ms`、`log_parameter_max_length=0`、`log_parameter_max_length_on_error=0`，避免参数进入日志；日志经 Docker/Promtail 进 Loki。
- 指标端点：API 在 `9100` 暴露 Prometheus 文本格式，仅 Compose 内网可达；Prometheus 每 15 秒抓取，保留 30 天。metric 标签只使用 `MatchedPath` 与有限枚举，不带 username/email/IP/request_id。
- 指标目录（最小集）：
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
- 告警目录（Prometheus 规则 → Alertmanager）：
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
- 告警投递：Alertmanager → 轻量 webhook 适配器 → 飞书自定义机器人 webhook（URL 在 secret 文件；适配器同时支持钉钉/企微/通用 webhook）。critical 立即通知，warning 聚合为每日摘要；通知内容包含环境、告警名、实例、开始时间、runbook 链接，不回传敏感值。
- 外部存活探测：使用外部 uptime 服务（Healthchecks.io / UptimeRobot / Better Stack 等）独立于部署主机监控 `ready` 与前端首页；主机整体故障时仍能告警。
- Dashboards：Grafana 提供最小三块：认证（登录结果、锁定、刷新复用）、API（QPS、错误率、延迟、连接池）、PostgreSQL/主机（连接、归档、磁盘、备份年龄）。
- 外部审计归档/WORM：v1 不做。审计表按 §8.5 分层保留，应用日志 14 天；HMAC 链与数据库权限继续提供数据库层篡改检测。触发条件（合规要求、审计事故）出现时再评估对象存储 Object Lock/外部 WORM。

### 10.7 维护任务与恢复/升级演练

- 调度：主机 systemd timer（`Persistent=true`、`RandomizedDelaySec`）每日触发一次性 Compose 任务；不引入常驻调度容器。备份 03:00 UTC，维护清理 04:00 UTC，避免重叠。
- 维护任务内容：
  - 删除过期/已撤销且超过 30 天的 `auth_refresh_tokens`；
  - 删除已撤销或过期超过 30 天的 `auth_sessions`（先删其 refresh token）；
  - 删除超过 30 天且未锁定的 `auth_login_failures` 与 `auth_login_throttle` 行；
  - 按 §8.5 的分层保留期批量删除 `audit_logs`，同一事务写入 `system_audit_logs_purged` 并更新 `audit_chain_head`；
  - 运行 `pgbackrest check`，导出备份年龄指标；
  - 所有任务先获取固定 key 的 `pg_try_advisory_lock`，未获得则退出 0；防多实例/重复触发并发执行。
- 恢复演练：每季度一次，按 §10.5 执行并记录耗时；失败必须开修复票据。
- 升级演练：每次 PostgreSQL 大版本升级前，用最近备份在临时实例预演 §11 的 `pg_upgrade` 步骤；演练通过才安排生产维护窗口。

### 10.8 CI（GitHub Actions）

- 触发：`pull_request` 与 `push main`；并发组按分支取消旧运行；生产镜像只在 `main` 与 `v*` tag 上推送。
- 后端 job：
  - `cargo fmt --all -- --check`；
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`；
  - `cargo test --workspace --all-targets`，使用 `postgres:18.6-alpine` service container（健康检查 `pg_isready`），`DATABASE_URL` 指向 service，`SQLX_OFFLINE=true` 编译期宏走 `.sqlx`；
  - `cargo sqlx prepare --check --workspace -- --all-targets --all-features`（sqlx-cli 0.9.0）；
  - 生成 OpenAPI 并与提交的 `openapi.json` 做 `git diff --exit-code` 防漂移；
  - 构建 API Docker 镜像（不推送）。
- 前端 job：
  - `oven-sh/setup-bun` 固定 1.4.2 + 缓存；`bun install --frozen-lockfile`；
  - `bun run lint`、`format:check`、`typecheck`、`i18n:check`、`test`、`build`；
  - `bunx playwright install --with-deps chromium`（browser mode 测试）；
  - orval 生成客户端后 `git diff --exit-code`；不兼容时按 §3 回退；
  - 构建 `web` Docker 镜像（`VITE_API_BASE_URL` 用占位值）。
- Compose 校验：`docker compose config -q`（含 `local-db`/`tools` profile）验证清单可解析；不启动完整栈。
- 镜像发布：`main` 推送 `ghcr.io/nasa-du/rs-admin-ds/api:sha-<short>` 与 `.../web:sha-<short>`；`v*` tag 额外推 `vX.Y.Z`；生产不使用 `latest`。
- 定时安全任务：每周 `cargo audit`、`bun audit`、Trivy 镜像扫描；PR 不因 advisory 数据库波动阻塞，CRITICAL 结果开 issue 并通知。CI 不接触生产 secret。
- 必需检查：上述 job 全部通过才允许合并；`main` 开启分支保护。

### 10.9 发布、回滚与迁移执行

- 镜像与审批：GitHub Actions 构建并推 GHCR 不可变 tag；部署由 `workflow_dispatch` 手动触发，绑定 `production` environment（required reviewer）。生产按 tag/digest 部署，不追 `latest`。
- 部署步骤（SSH 到部署主机）：
  1. `docker login ghcr.io`（只读 token 在主机 secret）；
  2. 记录当前 tag 到部署状态文件；更新 `.env` 的 `API_IMAGE` / `WEB_IMAGE`；
  3. `docker compose pull`；
  4. `docker compose run --rm migrate`（使用 `rs_admin_migrator` DSN，等待数据库就绪，应用全部 pending 迁移；非零退出则中止发布）；
  5. `docker compose up -d --remove-orphans`；
  6. 最多 120 秒等待 `/health/ready` 返回 200；失败则切回上一 tag 并 `up -d`，保留新 schema（expand/contract），人工决定是否恢复备份。
- 迁移策略：只前进、不自动回滚；迁移文件不可修改（SQLx checksum）；采用 expand/contract——先加列/表/索引，代码双读或默认值兼容，废弃列在后续版本删除；破坏性变更前先做一次 on-demand 备份。任何迁移必须先通过 CI 的空库与上一发布 schema 校验。
- 停机容忍：单 API 副本的发布预计数十秒不可用，计入 99.5% 月度预算；v1 不做蓝绿/双实例。
- 发布后 smoke：`/health/ready`、未认证 `GET /api/v1/auth/me` 返回 401 + ProblemDetails + CORS 预检、前端首页 200；不在生产创建测试用户或执行真实登录。
- 发布记录：tag、commit、迁移列表、回滚 tag、操作者与时间写入部署日志/issue；保留最近 10 个镜像 tag。

### 10.10 容量与 SLO

| 维度 | 假设 |
|---|---|
| 用户总量 | 10,000 |
| 并发已认证会话 | 50 |
| 峰值登录并发 | 20（Argon2 信号量串行/限并发） |
| 可用性 | 99.5%/月（计划维护 ≤ 4 小时/月，计入预算） |
| 延迟 | 业务 API p95 ≤ 300 ms；登录 p95 ≤ 1 s；刷新 p95 ≤ 200 ms |
| RPO / RTO | ≤ 15 分钟 / ≤ 2 小时 |
| 主机 | 4 vCPU / 8 GiB / 100 GiB SSD |

- 单机风险：主机是单点故障；99.5% 依赖主机稳定、外部 uptime 告警、异地备份与恢复演练。主机故障时 RTO ≤ 2 小时是目标而非保证。
- Argon2 内存：64 MiB × 并发上限 4 = 256 MiB；容器限制 1 GiB；登录峰值 20 并发在信号量队列中排队，超过 2 秒返回 503。禁止提高并发上限而不重新计算内存。
- 连接池：单副本 10 连接 + 迁移/维护/备份/监控预留 < `max_connections=100`；扩副本时按公式重算，必要时提升 `max_connections` 并评估 PG 内存。

### 10.11 多实例就绪 checklist

扩到 2+ API 副本时必须逐项验证：

- 无进程内会话/限流/锁定状态；全部共享状态在 PostgreSQL；
- refresh 轮换用 `SELECT ... FOR UPDATE`；清理/备份任务用 advisory lock 单执行；
- 网关能重新解析 `api` 的多个容器地址（nginx `resolver 127.0.0.11 valid=10s` + 变量 `proxy_pass`），不使用 sticky session；
- Prometheus 抓取每个实例，指标聚合不重复计数；
- 每副本 Argon2 并发 × 内存重新核算；日志中 `request_id` 可跨实例追踪。

扩容触发：用户 > 10,000、并发会话 > 50、登录 > 5 次/秒、数据库 > 50 GB、p95 持续超目标、主机 CPU > 70% 或内存 > 80% 持续 30 分钟；任一触发即复查主机规格/副本数/数据库规格，必要时迁移到多主机或托管 PG。

### 10.12 本地开发与 Compose 形态

- 默认开发：`docker compose --profile local-db up -d db` 只启动 PostgreSQL 18；API 用 `cargo run`，前端用 `bun dev`；Vite proxy 把 `/api/v1` 转发到 `http://localhost:8080`，使浏览器同源，避免本地 CORS/Cookie 噪音。
- 全栈联调：`docker compose --profile local-db --profile full up` 启动 `gateway`/`web`/`api`/`db`，用于发布前验证与 E2E；本地可用自签证书或 HTTP localhost，不得为此降低生产 Cookie 属性。
- `__Host-refresh` 本地行为：Chrome/Firefox 在 `http://localhost` 接受 `Secure` Cookie；若某浏览器拒绝，使用 `mkcert` 生成本地证书并以 `https://localhost:5173` 运行 Vite，**不**移除 `Secure`/`__Host-`。接入说明写入 README。
- 测试数据库：`#[sqlx::test]` 使用 Compose `db` 或 CI service；每个测试自动建库；开发 fixture 与生产种子严格分离。
- 密钥与 `.env`：`.env`、`/etc/rs-admin/secrets`、`*.pem`、`*.key` 均 gitignored；提供 `.env.example` 与密钥生成脚本；开发密钥可丢弃，生产密钥不进仓库。
- Docker 构建：`.dockerignore` 排除 `node_modules`、`dist`、`.git`、`target` 与本地缓存；前端构建阶段 `oven/bun:1.4.2`，运行时静态托管；后端多阶段构建，运行镜像不含 Rust 工具链。

### 10.13 运行手册（runbook）清单

实现阶段必须产出并在部署 issue 中链接：

1. 首次部署：主机准备、secret 生成/放置、Compose 启动、迁移、`bootstrap-super-admin`、smoke。
2. 发布与回滚：§10.9 的步骤、回滚 tag、失败处置。
3. 密钥轮换：JWT / refresh 派生 / throttle / 审计链；每类的发布顺序、保留时间与验证。
4. 备份与恢复：pgBackRest 全量/WAL、恢复演练、RTO 记录。
5. PostgreSQL 小版本/大版本升级：§11 的预检、演练、执行、回退。
6. 账号锁定/限流/审计告警的排障：指标、日志、SQL 只读查询、升级路径。
7. break-glass：`recover-super-admin` 的使用条件、双人确认、审计检查。
8. 证书续期与外部 uptime 检查。

---

## 11. PostgreSQL 18 升级路径

### 11.1 固定镜像与版本节奏

- `postgres:18.6-alpine` 为当前固定 tag（2026-11-07 核验）；小版本升级在 upstream 发布后 30 天内完成，步骤为改 tag → CI 通过 → 备份 → 重启 → 验证；不使用 `18-alpine` 浮动 tag。升级记录写入部署日志。
- 每次升级前重新核验 Docker Hub tag 与 pgBackRest 兼容矩阵；PG19 发布后按 §11.4 执行。

### 11.2 迁移可重复

- 迁移文件 UTC 时间戳前缀、提交到仓库、不可修改；`.sqlx` 离线元数据提交；CI 在空库与上一发布 schema 上各跑一次；`.gitattributes` 固定 `*.sql text eol=lf`。
- 迁移只前进；破坏性变更采用 expand/contract；不自动回滚。

### 11.3 允许的 PG18 特性

- 允许：`uuidv7()`、`pg_trgm`、`pg_stat_statements`。
- 不得使用未来 PG19-only 语法；schema 不承诺 PG17 回退。
- 生产使用 `scram-sha-256`；不使用 PG18 `oauth` HBA（SQLx 不支持）。

### 11.4 大版本升级（18 → 19）手册

1. **预检**：确认当前 18.x 为最新小版本；读取 PG19 release notes 与 pgBackRest 兼容矩阵；在临时实例用最近备份演练。
2. **备份**：生产全量备份 + WAL 归档完成并验证；记录当前 `_sqlx_migrations` 版本与扩展列表。
3. **维护窗口**：停止 API 与写入；停止 PG18。
4. **`pg_upgrade`**：使用包含 PG18 与 PG19 binaries 的固定维护镜像（镜像 tag 写入 runbook）；先 `pg_upgrade --check`，再执行升级。DB < 50 GB 时默认 `--copy` 以保留旧集群回退能力；磁盘/时间不足时才用 `--link` 并接受只能靠备份回退。
5. **启动 PG19**：运行 `ANALYZE`；验证扩展（`pg_trgm`、`pg_stat_statements`）、迁移表、审计链 HMAC、关键表行数；启动 API 并跑 smoke。
6. **回退**：`--copy` 时切回旧数据目录与 PG18 镜像；`--link` 时从备份恢复。验证通过后保留备份至少 30 天再删除旧集群。
7. **备选**：`pg_dumpall` / `pg_restore` 逻辑迁移（更简单、停机更久）；逻辑复制双写切换不在 v1 推荐范围。

### 11.5 演练

- 每次大版本升级前在同机临时实例演练完整流程。
- 每季度恢复演练与升级演练可合并执行；演练失败必须开修复票据，不得直接上生产。

---

## 12. 测试策略

### 12.1 分层与工具

| 层 | 范围 | 工具 | 运行位置 |
|---|---|---|---|
| Rust 单元 | domain 策略、密码策略、令牌派生、JWT claims、可信代理、审计 JCS/链、错误映射 | `cargo test` | 本地 + CI |
| Rust 集成 | 真实 PostgreSQL 18 + 事务/并发/迁移/限流/审计链 | `#[sqlx::test]` + testcontainers（本地）/ PG18 service（CI） | 本地 + CI |
| 契约 | OpenAPI 形状、错误码、ProblemDetails、生成客户端漂移 | utoipa 生成 + `git diff --exit-code`；前端 TS 类型检查 | CI |
| 前端单元 | `hasPermission`、redirect、auth store、单飞刷新、`runWithReauth`、i18n 映射 | Vitest | 本地 + CI |
| 前端集成 | 登录/冷启动/401/守卫/权限门控/Users/Roles/Audit/Settings | Vitest browser mode + MSW | 本地 + CI |
| E2E | 真实后端 + 真实浏览器 + 全栈 Compose | Playwright（browser mode 或独立 runner） | 发布前/CI 可选 |
| 性能 | Argon2 基准、登录/刷新/API p95、连接池饱和 | k6（或 oha） | 发布前/容量复查 |
| 安全 | 依赖审计、镜像扫描、头/CORS/Cookie/限流/链校验 | `cargo audit`、`bun audit`、Trivy、脚本化断言 | CI + 发布前 |
| 运维演练 | 备份恢复、PG 大版本升级、密钥轮换 | 手工 runbook + 验证脚本 | 每季度 |

### 12.2 Rust 单元测试

至少覆盖：

- 权限目录完整性：13 个 key、内置角色固定集、无重复、无运行时创建。
- 有效权限并集、超级管理员绕过、受限会话 allowlist。
- 授予子集规则：超管豁免、普通管理员子集通过/越界拒绝、自定义角色权限为空/非法。
- 管理范围：目标权限是否超出操作者、目标是否持有 `admin`/`super_admin`、self 可见、self 管理拒绝。
- 最后超管不变量：降级/停用最后一位、并发变更、自我降级。
- 用户名/邮箱规范化与唯一冲突；保留名；密码策略（长度、NFC、blocklist、等于标识、不 trim/不截断）。
- JWT：claims 集合、`exp - iat <= 15m`、60 秒 leeway、拒绝未知 `kid`/`none`/`jku`/`jwk`/`x5u`、`typ`。
- 刷新派生：初始随机、`counter+1`、HMAC 输入、grace 链现算、grace 外重放判定、密钥轮换窗口。
- 会话上限：第 6 次登录淘汰 `last_seen_at` 最旧、并发登录串行化。
- 可信代理：空 CIDR、单/多级代理、全可信回退、非法 XFF、IPv4-mapped IPv6、IPv6 /64。
- 限流阈值：5/10/15 次、24 小时衰减、IP 20/10min/15min、全局 200 告警/1000 熔断、锁定期间不增加计数。
- 审计：JCS 规范化、`row_hash` 链、`target_label` 映射、保留期分层、清理锚点、失败 fail-closed。
- 错误映射：每个 code 的 HTTP 状态、ProblemDetails 形状、`WWW-Authenticate`/`Retry-After`/`X-Request-Id`。

### 12.3 Rust 集成测试

至少覆盖：

- 迁移：空库全量迁移；上一发布 schema + 新迁移；`cargo sqlx prepare --check`；不可修改 checksum。
- 登录：成功、未知标识、错误密码、停用、临时密码过期、账号/IP/全局锁定、成功重置计数、dummy 验证时间均衡（粗粒度断言）。
- 刷新：正常轮换、30 秒 grace 幂等、grace 外重放撤销整族、会话 idle/absolute、用户停用/改密后的 refresh 失败。
- 会话：5 个上限淘汰、`last_seen_at` 60 秒节流、管理员撤销、logout/logout-all。
- 改密：当前密码错误、策略失败、成功后当前会话保留/其他撤销/新 token 生效/refresh 轮换。
- 临时密码：创建/重置一次性返回、72h 过期、受限会话 allowlist、过期后登录 403。
- 重新认证：300 秒窗口、成功重签、不轮换 refresh、失败计数、受限会话拒绝。
- RBAC：每个端点权限矩阵、管理范围列表/详情、self 操作、授予子集、特权角色、最后超管、角色软删除/409。
- API：分页/筛选/排序、`page_size>200`、`invalid_sort`、时间范围、未知字段、空 PATCH、ProblemDetails 字段。
- 审计：每个事件写入正确的 action/severity/target/details；详情访问写 `audit_log_accessed`；链校验；清理事件；运行角色无 UPDATE/DELETE。
- 并发：同一用户并发登录/刷新、同一 token 并发刷新（grace）、角色/停用并发、最后超管并发。
- 失败路径：数据库断开 503、审计写失败回滚业务、Argon2 信号量超时 503。

### 12.4 前端测试

至少覆盖：

- URL search 校验与同步、服务端分页/筛选/排序参数、页码越界回退。
- 权限门控与 403、自我操作禁用、角色入口的 `users.assign_role` + `roles.read` 双权限。
- step-up 取消保留表单、成功重试一次、5 秒竞态不二次弹窗。
- 创建（生成/手动密码）、一次性密码展示与关闭、密码不进入 cache。
- 编辑 dirty、`identifier_taken` 字段级/表单级回退、空对象不提交。
- 停用确认与 409 `last_super_admin`、启用、重置、解锁、撤销会话。
- 角色分配：基线保留、非可授予禁用、特权角色警告、权限子集/未知角色错误。
- 列表骨架/空态/错误态、`keepPreviousData`、无乐观更新。
- 错误码映射、未知 code 回退、`request_id`、429 倒计时。
- `/users?role_id=` 与 `/users?user=` 深链、角色筛选无 `roles.read` 时的精确 id 兜底。
- `UserPicker` 服务端搜索与精确 id 兜底。
- Roles：独立路由、单次保存 PATCH+PUT、部分失败提示、权限矩阵三态全选、非可授予禁用、内置只读、删除 409 跳转、未保存拦截。
- Audit：URL 同步详情 Sheet、每次打开重新 GET、绝对时间窗翻页不重复/漏行、筛选/排序/分页、结构化/原始 JSON、脱敏、未知枚举回退。
- Settings：Profile 只改 display_name、Account 改密/退出所有设备/语言、Appearance 本地、Notifications/Display 本地。
- i18n：语言解析与 Cookie、切换后 `<html lang>`/标题/文案、key 对齐、错误映射、日期/数字格式化、LTR。

### 12.5 E2E 与发布验收

在完整 Compose（`gateway`/`web`/`api`/本地 `db`）上运行最小 E2E：

1. 首次部署：迁移 → `bootstrap-super-admin` → 登录 → 强制改密 → Dashboard。
2. 管理员创建用户（生成临时密码）→ 新用户登录 → 改密 → 访问 Users。
3. 管理员创建自定义角色 → 分配权限 → 分配给用户 → 用户重新登录后菜单/路由按权限出现。
4. 管理员停用用户 → 该用户下一请求失效 → 重新登录 403 `account_disabled`；启用后可重新登录（旧会话不恢复）。
5. 管理员重置密码 → 目标用户旧会话失效 → 临时密码登录 → 强制改密。
6. 登录失败 5 次 → 429 `login_locked` + `Retry-After`；管理员解锁 → 可登录。
7. 审计页查询上述事件；每次打开详情写 `audit_log_accessed`。
8. 登出/全部登出；刷新页面冷启动；多标签广播登出。
9. 停用/重置/改密/撤销会话/刷新重放后，旧 access token 不再可用。

E2E 不在生产环境执行；发布后 smoke 只做非破坏性检查（§10.9）。

### 12.6 性能与容量测试

- Argon2 基准：在生产同规格硬件上测量 `m=64MiB/t=3/p=4` 的单次验证时间；>300ms 按 §8.1 回退并记录。
- 登录压测：20 并发登录，验证 p95 ≤ 1s、信号量排队、2 秒超时 503 的边界。
- API 压测：50 并发已认证会话访问列表/详情，验证 p95 ≤ 300ms、连接池 active/max < 0.8。
- 刷新压测：并发刷新同一会话，验证 grace 不误判、只有一个轮换、p95 ≤ 200ms。
- 长时间运行：24 小时稳态，观察连接池、限流表、审计链头竞争、日志量。

### 12.7 安全测试

- 依赖：`cargo audit`、`bun audit`、Trivy；CRITICAL 开 issue。
- 认证：未知用户/错误密码统一响应与时间均衡；锁定/限流；dummy hash；密码策略；临时密码一次性。
- 会话：Cookie 属性、`__Host-`、无 `Domain`、CORS 精确 Origin、CSRF 头、401 刷新单飞、grace 外重放。
- 头与 CSP：API 头、前端 CSP、HSTS、无 `unsafe-inline` script。
- 审计：写入失败 fail-closed、运行角色无 UPDATE/DELETE、链重算、清理锚点。
- 日志：密码/令牌/Cookie/Authorization/密钥不出现在应用日志、审计、metrics 标签。
- 权限：越权矩阵（普通管理员对超管用户、角色子集、自我操作、受限会话、审计范围）。
- 注入/输入：JSON 未知字段、超长密码、Unicode/NFC、用户名保留名、SQL 注入（SQLx 参数化）。
- 数据泄露：错误响应不回传 SQL/堆栈/内部细节；`request_id` 可追踪但不含 PII。

### 12.8 运维演练

- 备份恢复：每季度从异地备份恢复，验证迁移版本、关键表行数、审计链；记录 RTO。
- PG 大版本升级：每次升级前在临时实例演练 §11.4。
- 密钥轮换：按 §8.7 演练一次 JWT/refresh/throttle/审计链轮换；验证旧令牌/旧审计链仍可验证。
- 故障注入：数据库不可用 → 503 fail-closed；审计写入失败 → 业务回滚；网关 5xx → 前端错误页/重试。

### 12.9 测试数据与 fixture

- 生产不 seed 任何用户；开发/测试 fixture 与生产种子严格分离。
- 测试用户、角色、权限只通过测试工厂创建；密码使用固定测试值但不得进入生产日志。
- 审计链测试使用固定密钥环与固定时钟/序列，保证可重复；真实时间测试使用数据库 `now()`。
- 前端 MSW fixture 与后端 schema 对齐；contract 测试防止漂移。

### 12.10 覆盖率与质量门

- 不设全仓覆盖率数字；domain/security/audit 的关键策略必须有单元测试，auth/RBAC/API 的关键路径必须有集成测试。
- 每个 §14 验收标准至少映射到一个自动化测试或发布验收步骤；无法自动化的（如浏览器 Cookie 兼容性、恢复演练）列入发布 checklist 并记录结果。
- 任何新增错误码/事件/权限必须同时更新 OpenAPI、前端映射、审计目录与测试。

---

## 13. 分阶段里程碑

依赖顺序：M0 → M1 → M2 → M3 → M4 → M5 → M6。前端 M3 可在 M1 契约稳定后并行；M4 依赖 M2 API 与 M3 基础设施；M5 可在 M2 后并行准备；M6 收口。

### M0 — 基础骨架与契约

范围：

- 仓库布局、Cargo workspace、`rust-toolchain.toml`、`.env.example`、`.gitattributes`。
- 前端 fork 初始化、bun 迁移、依赖裁剪、LICENSE/THIRD-PARTY-NOTICES。
- 后端 config、日志、request_id、错误类型、ProblemDetails、健康检查、metrics recorder。
- 迁移框架、`_sqlx_migrations`、`.sqlx`、CI 骨架（fmt/clippy/build/compose config）。
- 权限目录、内置角色定义、domain 策略骨架（无 DB）。
- OpenAPI 骨架 + orval 生成流水线。

退出标准：

- `cargo build`、`bun install --frozen-lockfile`、`bun run build` 通过。
- `docker compose config -q` 通过。
- 健康检查 `live`/`ready` 可运行（ready 查库）。
- 权限目录与错误码目录有单元测试；OpenAPI 生成物可提交。

### M1 — 认证核心

范围：

- users/auth_sessions/auth_refresh_tokens/auth_login_failures/auth_login_throttle 迁移。
- Argon2id、密码策略、dummy hash、JWT EdDSA、刷新派生/grace/reuse、会话上限、idle/absolute。
- `login`/`refresh`/`logout`/`logout-all`/`me`/`change-password`。
- 三层限流、可信代理、审计写入器 + 链哈希、CLI bootstrap/recover。
- 认证端点契约、错误码、OpenAPI、集成测试。

退出标准：

- 登录/刷新/登出/改密/重新认证（端点可先占位）/受限会话的集成测试通过。
- 停用/重置/改密后旧 token 失效；grace 外重放撤销整族。
- 限流阈值与锁定测试通过；审计事件与链校验通过。
- `bootstrap-super-admin` 在空库可用；`recover-super-admin` 可用。
- 性能基准记录 Argon2 单次耗时。

### M2 — RBAC 与管理员 API

范围：

- roles/permissions/role_permissions/user_roles 迁移；启动目录同步。
- 权限解析、授予子集、管理范围、自我操作、最后超管不变量。
- Users CRUD、Roles CRUD、Permissions、Audit 查询、step-up、错误码。
- 审计事件全目录、target 映射、详情访问。
- 集成测试覆盖授权矩阵与并发边界。

退出标准：

- §6.5 端点授权矩阵全部有测试。
- 越权/范围/自我/最后超管/角色软删除测试通过。
- `GET /roles` flags、`GET /permissions` grantable、审计查询过滤/分页/时间范围通过。
- OpenAPI 与前端生成客户端对齐（可先生成占位页面）。

### M3 — 前端基础与认证

范围：

- 路由树、侧栏、命令菜单、页面裁剪、路由守卫。
- auth store、bootstrap、API client、401 单飞、重新认证 helper。
- 登录页、强制改密、错误页、403/404/500/503、Settings 骨架。
- i18n 基础设施、错误码映射、格式化 helper、locale Cookie、文档标题。
- 前端单元/集成测试与 CI。

退出标准：

- 未登录不能进入受保护页；已登录访问 `/sign-in` 回到目标；`redirect` 不能跳出站内。
- 冷启动 refresh 成功/401/网络失败；并发 401 只刷新一次；刷新失败只登出一次。
- 重新认证对话框单飞/取消/只重试一次；5 秒竞态不二次弹窗。
- 权限门控隐藏/禁用语义正确；403 页面可用。
- 默认 zh-CN，切换 en-US 全量更新并持久化；key 对齐检查通过。

### M4 — 管理 UI

范围：

- Users 页面：列表/筛选/URL/深链、创建/编辑/启停/重置/解锁/角色分配/撤销会话、一次性密码、无删除/批量。
- Roles 页面：列表/创建/详情/编辑、权限矩阵、单次保存 PATCH+PUT、部分失败、删除 409 跳转、未保存拦截。
- Audit 页面：列表/筛选/绝对时间窗/详情 Sheet/访问留痕/脱敏/双视图。
- Settings 五子页、Account 改密/退出所有设备/语言。
- 前端集成测试与 E2E 前置脚本。

退出标准：

- §9.8–9.11 的页面行为与错误矩阵有测试。
- `UserPicker`/`RoleMultiSelect` 共享组件可用；双权限门控正确。
- 审计详情每次打开重新 GET 并写 `audit_log_accessed`；绝对时间窗翻页不重复/漏行。
- Roles 单次保存与部分失败提示；内置角色只读；删除 409 链接可用。

### M5 — 部署、可观测与 CI

范围：

- Compose（gateway/web/api/db/maintenance/backup/observability）、主机加固、secret 文件、TLS/certbot、nginx 配置。
- 备份/恢复、维护任务、systemd timer、Prometheus/Alertmanager/Grafana/Loki、外部 uptime。
- CI 完整流水线、GHCR、发布/回滚、迁移执行、runbook。
- 容量/SLO 与多实例 checklist。

退出标准：

- `docker compose --profile local-db --profile full up` 全栈可用；`/health/ready`、CORS、静态站、CSP 正确。
- 备份/恢复演练一次；RPO/RTO 记录。
- CI 后端/前端/Compose/镜像 job 通过；orval/OpenAPI 无漂移。
- 告警规则可触发并有 runbook；外部 uptime 配置完成。
- 发布流程 dry-run：迁移 → 部署 → ready → 回滚上一 tag。

### M6 — 硬化与验收

范围：

- 全量 E2E、性能/容量、安全测试、依赖审计、故障注入。
- 文档收口：本规格、ADR 索引、runbook、README、CONTRIBUTING、部署检查清单。
- 验收矩阵执行与缺陷修复。

退出标准：

- §14 全部验收标准通过或有明确记录的风险接受。
- 性能目标达成；Argon2 参数最终确定并记录。
- 恢复/升级/密钥轮换演练完成。
- 安全评审记录有意偏离（持久 Cookie、管理员临时密码、审计链边界、单机）。
- 交付物齐备：本规格、8 份 ADR、CONTEXT、研究文档、OpenAPI、生成客户端、runbook。

---

## 14. 验收标准

以下标准必须可测试。每条至少映射到 §12 的一个测试层或发布验收步骤。状态码与错误码以 §7 为准。

> M6 的执行记录与逐条证据见 [`docs/acceptance/matrix.md`](../acceptance/matrix.md)
> 与 [`docs/acceptance/2026-11-11.md`](../acceptance/2026-11-11.md)。
> 性能、安全、故障注入与依赖/镜像扫描的可复现入口见
> [`docs/acceptance/README.md`](../acceptance/README.md)。

### 14.1 认证与会话

| ID | 验收标准 | 测试层 |
|---|---|---|
| AC-AUTH-01 | 正确 username 或 email + 密码登录返回 200、15 分钟 access token、`UserSelf`、`__Host-refresh` Cookie（`HttpOnly; Secure; SameSite=Strict; Path=/`，无 `Domain`，`Max-Age=43200`），并写 `authn_login_success`。 | 集成 + E2E |
| AC-AUTH-02 | 错误密码与未知标识都返回 401 `invalid_credentials`，响应文案/状态一致；未知标识执行 dummy Argon2；两者都写 `authn_login_fail`，不泄露用户是否存在。 | 集成 + 安全 |
| AC-AUTH-03 | 账号层连续失败 5/10/15 次分别锁定 30 秒/5 分钟/1 小时；锁定时返回 429 `login_locked` + `Retry-After`，仍执行 dummy Argon2，锁定期间不增加计数；成功登录后计数清零。 | 集成 |
| AC-AUTH-04 | IP 层 20 次失败/10 分钟锁 15 分钟并返回 429 `rate_limited`；全局层 1000 次/5 分钟熔断 60 秒；熔断不影响 refresh、业务 API 与已登录会话；成功登录不重置 IP 计数。 | 集成 + 性能 |
| AC-AUTH-05 | 停用用户后，其已有会话在下一受保护请求返回 403 `account_disabled`（若 access token 已过期，refresh 也返回 403 `account_disabled`），不能继续访问任何业务 API；重新登录返回 403 `account_disabled`；重新启用不恢复旧会话。 | 集成 + E2E |
| AC-AUTH-06 | refresh 轮换：正常刷新返回新 access token 与轮换 Cookie；旧 refresh token 在 30 秒内重试幂等返回同一后继；grace 外重放撤销整族并写 `authn_refresh_reuse_detected`（critical），返回 401 `refresh_reuse_detected`。 | 集成 |
| AC-AUTH-07 | 会话 idle 30 分钟、absolute 12 小时；第 6 次登录按 `last_seen_at` 淘汰最旧会话并写 `authn_session_evicted`；`last_seen_at` 更新最多每 60 秒一次。 | 集成 |
| AC-AUTH-08 | logout 只撤销当前会话，204 且幂等；logout-all 撤销全部会话 + `token_version++` + 清 Cookie；两者都写对应审计。 | 集成 + E2E |
| AC-AUTH-09 | 自助改密：当前密码错误 422 `invalid_current_password`；策略失败 422；成功后当前会话保留并重签 access token、其他会话撤销、`token_version++`、refresh 轮换；旧 access token 下一请求失效。 | 集成 + E2E |
| AC-AUTH-10 | 临时密码：创建/重置响应只返回一次；`must_change_password=true`、72h 过期；受限会话只允许 me/refresh/change-password/logout，其他端点 403 `password_change_required`；过期后登录 403 `temporary_password_expired`。 | 集成 + E2E |
| AC-AUTH-11 | 敏感操作在 `authenticated_at` 超过 300 秒时返回 403 `reauth_required`（`details.max_age=300`）；重新认证成功重签 access token、不轮换 refresh、不建新会话；失败计入账号/IP 计数；受限会话 403 `password_change_required`。 | 集成 + 前端集成 |
| AC-AUTH-12 | 每请求认证顺序为 JWT → 用户状态 → 会话 → token_version → 权限 → 范围 → step-up；数据库不可用返回 503 `service_unavailable`，不得放行。 | 集成 + 故障注入 |

### 14.2 RBAC 与授权

| ID | 验收标准 | 测试层 |
|---|---|---|
| AC-RBAC-01 | 权限目录恰为 §6.1 的 13 个 key；内置角色权限集固定；`super_admin` 绕过授权但认证/会话/step-up 仍适用；`user` 为不可移除基线。 | 单元 + 集成 |
| AC-RBAC-02 | 有效权限为多角色并集；角色/权限变更在下一请求生效；RBAC 变更不递增 `token_version`、不撤销会话。 | 集成 |
| AC-RBAC-03 | 非超管创建/修改/分配角色时，目标权限集超出自身有效权限返回 403 `permission_subset_violation`；超管豁免。 | 集成 |
| AC-RBAC-04 | 非超管只能读取/管理权限不超出自身且不持有 `admin`/`super_admin` 的用户；`GET /users` 按范围过滤且 `total` 为过滤后数量；范围外详情/管理动作 403 `out_of_scope`；self 始终可见。 | 集成 |
| AC-RBAC-05 | 管理动作作用于自己返回 403 `self_action_forbidden`；自我停用/降级/重置/角色分配/撤销会话/解锁均被拒绝。 | 集成 + 前端集成 |
| AC-RBAC-06 | `admin`/`super_admin` 角色的授予/撤销仅超级管理员；普通管理员提交时 403 `permission_subset_violation` 或等价拒绝；角色选择器对应项禁用。 | 集成 + 前端集成 |
| AC-RBAC-07 | 最后一位启用的超级管理员不能被停用或降级；违反 409 `last_super_admin`；并发变更不会产生零超管。 | 集成 |
| AC-RBAC-08 | 角色分配整体替换；基线 `user` 始终保留；未知/软删除角色 422 `unknown_role`；角色变更写 `user_roles_changed`（涉及特权角色 critical）。 | 集成 |
| AC-RBAC-09 | 自定义角色创建/修改/删除：名称活跃唯一、空权限 422 `empty_permissions`、非法权限 422 `invalid_permission`、内置写 403 `role_immutable`、仍有分配删除 409 `role_in_use`、软删除后列表/候选隐藏且详情 404。 | 集成 |
| AC-RBAC-10 | `GET /roles` 不按范围隐藏且返回按操作者计算的 `assignable`/`manageable`；`GET /permissions` 返回完整目录与 `grantable`；不新增 `assigned_user_count`/`deletable`。 | 集成 + 前端集成 |
| AC-RBAC-11 | `GET /audit-logs` 持有 `audit.read` 即可查看全部，不按范围过滤；详情每次打开写 `audit_log_accessed`；列表查询不写。 | 集成 + 前端集成 |
| AC-RBAC-12 | 越权矩阵：无权限端点 403 `forbidden`；角色/用户越权 403；受限会话 403 `password_change_required`；未认证 401；不存在 404；不使用 403 伪装 404 或反之。 | 集成 + 安全 |

### 14.3 API、数据与安全

| ID | 验收标准 | 测试层 |
|---|---|---|
| AC-API-01 | 所有错误为 `application/problem+json`，含 `type`/`code`/`title`/`status`/`detail`/`instance`/`request_id`；`X-Request-Id` 一致；401 带 `WWW-Authenticate`；429 带 `Retry-After`；不回传 SQL/堆栈。 | 契约 + 集成 |
| AC-API-02 | 分页默认 20/上限 200，`page_size>200` 422 `page_size_too_large`；排序白名单外 422 `invalid_sort`；筛选/排序/分页按端点契约工作。 | 集成 |
| AC-API-03 | 写请求未知字段 400 `unknown_field`；空 PATCH 422 `empty_update`；ID/时间/枚举格式符合 §7.1；创建 201 + Location；删除/登出/撤销 204。 | 集成 + 契约 |
| AC-API-04 | CORS 只允许精确 Origin，不允许 `*`/`null`；credentials 仅 §7.11 端点；`Vary: Origin`；4xx/5xx 也带 CORS 头；预检 204。 | 集成 + 安全 |
| AC-API-05 | CSRF：login/refresh/logout/logout-all 缺 Origin/`X-Requested-With`/JSON 时 403 `csrf_failed`；业务 Bearer 端点不读 Cookie；refresh Cookie 属性精确。 | 集成 + 安全 |
| AC-API-06 | 安全头：API 所有响应含 `no-store`/`nosniff`/`no-referrer`/`DENY`/CSP；前端 CSP 无内联 script；HSTS 与 HTTP→HTTPS 生效；`/metrics`/`/health/*`/`/api-docs` 不暴露公网。 | 部署 + 安全 |
| AC-API-07 | 审计写入与业务同事务，失败 fail-closed；审计链可用固定密钥重算；运行角色对 `audit_logs` 无 UPDATE/DELETE；清理写 `system_audit_logs_purged` 并保留锚点。 | 集成 + 安全 |
| AC-API-08 | 审计保留：info/warning ≥180 天、error/critical ≥365 天；查询默认 30 天、最大 90 天；`invalid_time_range`/`time_range_too_large` 正确。 | 集成 + 运维 |
| AC-API-09 | 密码哈希：Argon2id PHC、参数按 §8.1、登录成功 rehash；密码策略 15–128/NFC/blocklist/不等于标识；所有失败返回正确 code。 | 单元 + 集成 |
| AC-API-10 | 启动配置校验：secret 文件/PEM/kid/iss/aud/密钥长度/CIDR/Origin/数据库 TLS 失败即退出；错误信息不含密钥值。 | 集成 + 部署 |
| AC-API-11 | 日志/审计/metrics 不出现密码、哈希、临时密码、令牌、Cookie、Authorization、密钥；username/email 不进应用日志。 | 安全 |
| AC-API-12 | OpenAPI 生成物与提交的 `openapi.json` 一致；orval 生成客户端与提交物一致（`git diff --exit-code`）；operationId/schema/错误码/头齐全。 | 契约 + CI |

### 14.4 前端

| ID | 验收标准 | 测试层 |
|---|---|---|
| AC-FE-01 | 登录页只有 identifier + password；无 sign-up/forgot/OTP/社交/无效链接；错误按 code 映射；429 倒计时禁用提交；`request_id` 可复制。 | 前端集成 + E2E |
| AC-FE-02 | 冷启动 `bootstrap()` 单飞：refresh → me；refresh 401 静默未登录；网络/5xx 不误判登出；403 `account_disabled` 清会话并提示。 | 前端集成 |
| AC-FE-03 | 并发 401 只触发一次 refresh；原请求只重试一次；refresh 失败只登出一次；不按 `expires_in` 主动刷新。 | 前端单元 + 集成 |
| AC-FE-04 | 路由守卫：未登录跳 `/sign-in?redirect=`；`must_change_password` 只能先改密；无权限跳 `/403?from=`；公开错误页不套守卫；`redirect` 不能跳出站内。 | 前端集成 + E2E |
| AC-FE-05 | 权限门控：无动作权限隐藏按钮/菜单；目标级不可管理禁用 + tooltip；路由权限集中配置；直接访问无权限显示 403。 | 前端集成 |
| AC-FE-06 | 重新认证：403 `reauth_required` 只弹一次；成功后只重试一次；取消保留表单不执行；5 秒竞态不二次弹窗；密码错误/429 在对话框内提示。 | 前端单元 + 集成 |
| AC-FE-07 | Users：列表/筛选/排序/分页/URL 同步；创建/编辑/启停/重置/解锁/角色分配/撤销会话；无删除/批量；临时密码只显示一次且不进 cache；双权限角色入口；深链 `?user=`/`?role_id=`。 | 前端集成 + E2E |
| AC-FE-08 | Roles：独立路由；权限矩阵完整目录、三态全选、非可授予禁用；单次保存 PATCH+PUT；部分失败提示；内置只读；删除 409 跳转；未保存拦截。 | 前端集成 + E2E |
| AC-FE-09 | Audit：列表/筛选/排序/分页 URL 同步；默认 30 天绝对窗口；详情 Sheet 每次打开重新 GET 并留痕；结构化/原始 JSON；精确 key 脱敏；未知枚举回退；无导出/编辑。 | 前端集成 + E2E |
| AC-FE-10 | Settings：Profile 只改 display_name；Account 改密/退出所有设备/语言；Appearance/Notifications/Display 本地偏好；不接后端。 | 前端集成 |
| AC-FE-11 | i18n：默认 zh-CN，en-US 可切换并持久化；`<html lang>`/标题/文案/日期/数字更新；key 对齐检查通过；错误码/审计枚举回退正确；LTR-only 无手动切换。 | 前端单元 + 集成 |
| AC-FE-12 | 前端构建：`bun install --frozen-lockfile`、`bun run check`、`bun run build` 通过；CI/Docker 无 pnpm；LICENSE/THIRD-PARTY-NOTICES 齐全；生成客户端无漂移。 | CI + 部署 |

### 14.5 部署、可观测与运维

| ID | 验收标准 | 测试层 |
|---|---|---|
| AC-OPS-01 | Compose 全栈启动：gateway/web/api/db；端口只在预期位置暴露；容器非 root、API 只读根文件系统、`no-new-privileges`；`/health/ready` 通过。 | 部署 |
| AC-OPS-02 | TLS/HSTS/certbot 续期；HTTP 跳 HTTPS；CSP 与缓存头正确；SPA fallback 可用；生产不暴露 metrics/health/api-docs。 | 部署 + 安全 |
| AC-OPS-03 | CI：后端 fmt/clippy/test/sqlx prepare/OpenAPI；前端 lint/format/typecheck/i18n/test/build/orval；Compose config；镜像构建全部通过；main 分支保护。 | CI |
| AC-OPS-04 | 发布：手动审批 → 拉镜像 → 一次性 migrate → up → 120 秒 ready；失败回滚上一 tag；迁移只前进/expand-contract。 | 发布 + 运维 |
| AC-OPS-05 | 备份：每日全量 + WAL 归档，RPO ≤15 分钟；保留 30 天；备份失败告警；恢复演练记录 RTO ≤2 小时。 | 运维 |
| AC-OPS-06 | 可观测：指标目录齐全；告警规则可触发；外部 uptime 独立探测；日志脱敏；Grafana 三块看板可用。 | 运维 |
| AC-OPS-07 | 维护任务：清理 refresh token/session/login failures/throttle/audit；advisory lock 单执行；审计清理写事件与锚点。 | 运维 + 集成 |
| AC-OPS-08 | 多实例 checklist 全部满足；扩容触发条件有监控；容量假设与 SLO 记录。 | 运维 + 评审 |
| AC-OPS-09 | PG18：固定 `18.6-alpine`；小版本升级流程；大版本 18→19 `pg_upgrade` 演练、`--copy` 默认、回退路径与扩展/审计链验证。 | 运维 |
| AC-OPS-10 | runbook 齐备：首次部署、发布回滚、密钥轮换、备份恢复、PG 升级、告警排障、break-glass、证书续期。 | 文档评审 |

### 14.6 规格与交接

| ID | 验收标准 | 测试层 |
|---|---|---|
| AC-DOC-01 | 本规格覆盖 §1–§15 全部主题；所有引用 ADR/票据/研究文档存在；术语与 `CONTEXT.md` 一致。 | 文档评审 |
| AC-DOC-02 | §15 的矛盾消解表覆盖发现的冲突；假设清单完整；无未决问题留白（每个条件化方案有触发条件与默认选择）。 | 文档评审 |
| AC-DOC-03 | 权限矩阵、错误码目录、审计事件目录、撤销矩阵、验收标准可直接被实现与测试使用；无重复/遗漏。 | 文档评审 |
| AC-DOC-04 | 8 份 ADR 与 `CONTEXT.md` 已提交；本规格在 `docs/specs/login-system.md`；后续实现 PR 必须引用对应验收 ID。 | 文档评审 |

---

## 15. 矛盾消解、假设与条件化方案

### 15.1 矛盾消解

以下冲突来自早期票据评论与后续 ADR/Amendments。按 §0 的权威顺序，**较晚的 ADR 修订优先**。

| # | 冲突 | 早期表述 | 最终决议 | 依据 |
|---|---|---|---|---|
| 1 | utoipa 版本 | 地图 ticket 8 gist 写 “utoipa 3.1” | **utoipa 6.0.0 + utoipa-axum 0.3.0** | 研究文档 §1/§2；ADR-0004 §4。地图 gist 为笔误，已修正 |
| 2 | refresh grace 实现 | 票据 5：AES-GCM 保存 `grace_token_ciphertext` | **派生式 HMAC 轮换**；数据库只存 SHA-256 | ADR-0001 2026-11-05 Amendment；ADR-0004 §8 |
| 3 | 会话淘汰顺序 | 票据 5：撤销 `created_at` 最早的会话 | **按 `last_seen_at` 撤销最久未活动会话** | ADR-0001 2026-11-05 Amendment |
| 4 | 会话表字段 | 票据 5：`initial_ip`/`last_ip`/`initial_user_agent`/`last_user_agent`/`device_label` | **ADR-0004 的 `ip`/`user_agent`/`last_seen_at`/`idle_expires_at`/`absolute_expires_at`/`authenticated_at`** | ADR-0004 §8；ADR-0005 §10 |
| 5 | refresh token 表 | 票据 5：`id` 主键 + `replaced_by` UUID | **`token_hash` 主键 + `rotation_counter` + `replaced_by_hash`** | ADR-0004 §8 |
| 6 | 状态端点动词 | 票据 6/7：`POST /users/{id}/status` | **`PUT /users/{id}/status`** | ADR-0004 §7 |
| 7 | 撤销会话端点 | 票据 6：`POST /users/{id}/sessions/revoke` | **`DELETE /users/{id}/sessions`** | ADR-0004 §7 |
| 8 | 自助资料端点 | 票据 7：`PATCH /me` | **`PATCH /auth/me`** | ADR-0004 §6 |
| 9 | `GET /permissions` 权限 | 票据 6：需要 `roles.read` | **`roles.read` / `roles.create` / `roles.update` 任一** | ADR-0004 §7 |
| 10 | 权限目录与 `admin` | 票据 6 初版未含 `users.unlock` | **13 项包含 `users.unlock`；`admin` 拥有** | ADR-0002 2026-11-05 Amendment；ADR-0003 |
| 11 | 审计事件命名 | 票据 5/研究：`authn_token_reuse`、`authn_password_change` | **`authn_refresh_reuse_detected`、`authn_password_changed`** | ADR-0005 §6.1 |
| 12 | 审计 target 形状 | 票据 8：`{type,id}` | **`{type,id,label}` + `target_label` 列** | ADR-0004/0005 2026-11-07 Amendments |
| 13 | 审计查询窗口 vs 保留期 | 票据 8：默认 30 天/最大 90 天；票据 9：保留 180/365 天 | **查询窗口 30/90 天；保留期 180/365 天**（不矛盾：查询窗口限制单次检索，保留期限制数据寿命） | ADR-0004 §7；ADR-0005 §6.3 |
| 14 | CORS credentials 清单 | ADR-0005 §7.4 原文：仅 login/refresh/logout | **增加 change-password 与 logout-all** | ADR-0005 2026-11-07 Amendment |
| 15 | 本地开发 Cookie | 票据 5：本地可配置关闭 `Secure` | **不移除 `Secure`/`__Host-`；优先 `https://localhost` 或 mkcert** | ADR-0008 §12.3；本规格 §10.12 |
| 16 | 会话清理 | 票据 5：会话过期后保留 30 天再删除；ADR-0004/0008 维护任务未列 session 删除 | **保留 30 天；维护任务先删 refresh token 再删 session**（满足 RESTRICT） | 本规格 §4.5/§10.7，补齐 ADR 空白 |
| 17 | 审计 `target_label` 与链哈希 | 票据 11 新增 `target_label`；ADR-0005 原链哈希未含该列 | **`target_label` 纳入 JCS 行规范化与 HMAC 链** | ADR-0005 2026-11-07 Amendment |
| 18 | 审计排序 | 票据 8 通用排序；票据 11 要求仅 `occurred_at` | **审计列表排序白名单仅 `occurred_at`** | ADR-0004 2026-11-07 Amendment |
| 19 | 前端 403 去留 | 研究审计建议删除 `/403` | **保留真实 `/403` 页面与组件** | 票据 10/12/13 交叉约束；本规格 §9.1 |
| 20 | RTL | 上游有手动 LTR/RTL 切换 | **v1 仅 LTR；保留基础设施，移除手动切换** | ADR-0007 §7；ADR-0006 §8 Amendment |
| 21 | 语言持久化 | 票据 12 只说“本地偏好” | **Cookie `locale`，非 localStorage；不接后端** | ADR-0007 §4 |
| 22 | 审计详情访问 | 票据 11 要求每次打开留痕；ADR-0005 事件表有 `audit_log_accessed` | **每次打开重新 GET 并写事件；列表不写** | ADR-0005 §6.1；票据 11 |
| 23 | 创建用户 step-up | 票据 7 未明确；ADR-0005 要求 `POST /users` 一律 step-up | **一律 step-up** | ADR-0005 §5.1 |
| 24 | 解锁 step-up | 票据 7 未明确；ADR-0005 明确不要求 | **不要求 step-up** | ADR-0005 §5.2 |
| 25 | 审计 `request_id` 类型 | ADR-0004 示例字符串；ADR-0005 说 UUIDv7 | **数据库 `uuid`；API 字符串** | ADR-0005 §6.2；本规格 §4.2 |
| 26 | 数据库 FK 删除策略 | 票据 5 初版 `auth_refresh_tokens ... ON DELETE CASCADE` | **一律 `RESTRICT`；清理按顺序显式删除** | ADR-0004 §8；本规格 §4.5 |
| 27 | 会话淘汰审计命名 | 票据 5 `excess_sessions_exceeded` | **审计 `authn_session_evicted`；revoke_reason `excess_sessions`** | ADR-0005 §6.1；本规格 §5.3 |
| 28 | 刷新重放是否 `token_version++` | 票据 5 只说撤销整族；未说全用户 | **仅撤销该会话/令牌族，不递增 `token_version`** | ADR-0001 §3；本规格 §5.2 |
| 29 | 目标资格口径 | ADR-0004 只给 `assignable`/`manageable` 字段 | **目标资格口径：不含操作者动作权限；前端再与 `/me` 求交集** | 票据 10 §3；本规格 §6.4 |
| 30 | 用户列表范围 | 票据 6 初版说 `users.read` 不受目标范围限制 | **列表按管理范围过滤；self 可见；详情范围外 403** | ADR-0002 2026-11-05 Amendment；ADR-0004 §7 |

### 15.2 假设清单

| # | 假设 | 影响 | 验证/触发条件 |
|---|---|---|---|
| A1 | 生产前端与 API 可同站不同源（`admin.example.com` + `api.example.com`） | 决定 refresh Cookie 是否为第一方、CORS 与 CSP | 上线前 DNS/域名检查；无法同站 → BFF（ADR-0001） |
| A2 | 实际域名、证书、网关归属未定，规格使用占位 | 配置/CSP/HSTS/certbot 需替换 | 部署票据 §10.2 检查清单 |
| A3 | Argon2 生产基准 100–300 ms 可达成 | 决定最终参数 | 实现首日 benchmark；>300ms 按 §8.1 回退 |
| A4 | orval 8.46 可在 Bun 1.4.2 + Node ≥22.18 下运行 | 决定客户端生成方案 | 实现首日验证；不兼容 → `openapi-typescript` + `openapi-fetch` |
| A5 | 前端上游快照 `e16c87f` 是裁剪基线 | 后续上游变更不自动合并 | 实现时锁定快照；需要升级时单独评估 |
| A6 | 系统无邮件服务，临时密码线下交付 | 登录/重置/找回流程 | 产品约束；引入邮件服务 → 新 ADR 重新评估邀请/重置链接 |
| A7 | 单台主机、单 API 副本、99.5% 可用性 | 部署/容量/发布停机 | 容量触发条件出现 → 多实例/托管 PG |
| A8 | 容量 10,000 用户 / 50 并发会话 / 20 峰值登录 | 连接池/内存/SLO | 超限 → §10.11 扩容复查 |
| A9 | 不引入 Redis；PostgreSQL 保存全部共享状态 | 限流/会话/锁定/审计 | 若数据库压力需要缓存 → 5–15 秒缓存并文档化撤销窗口 |
| A10 | 现代 evergreen 浏览器；Chrome/Safari 无痕可用 | Cookie/CORS/前端行为 | E2E/兼容性测试；Safari 拒绝 Cookie → 检查同站/BFF |
| A11 | 密码 blocklist 来源可选用兼容许可证（HIBP 频率列表或 SecLists top-100000） | 构建内嵌列表 | 实现首日确认许可证；不兼容 → 等价来源 |
| A12 | 生产使用 SCRAM + `sslmode=verify-full`；不使用 PG18 `oauth` HBA | 数据库连接 | 启动校验；外部数据库运维方需支持 |
| A13 | 外部数据库（若使用）由运维方负责备份/补丁/大版本升级 | RPO/RTO 责任边界 | `DATABASE_URL` 指向外部时在部署记录中注明 |
| A14 | 审计链密钥环包含当前 key 与旧钥；旧钥保留 ≥365 天 | 历史审计可验证 | 启动校验；轮换 runbook |
| A15 | 单次请求 30 秒超时、认证 1 KiB body、其他 64 KiB | 网关/API 行为 | 压测与边界测试 |
| A16 | `trusted_proxy_cidrs` 覆盖 Compose 子网；网关覆盖 XFF | 客户端 IP/限流/审计 | 启动告警；部署检查 |
| A17 | 前端 locale 仅 `zh-CN`/`en-US`；默认 zh-CN | i18n 资源/测试 | 产品约束；新增语言需评估包体与 RTL |
| A18 | 浏览器本地时区用于展示；API 一律 UTC | 审计/时间筛选 | `Intl` 测试；时区标注 |
| A19 | 生产不使用 `latest` 镜像 tag；发布按 digest/tag | 可复现/回滚 | CI/发布流程 |
| A20 | 无 staging 环境；用全栈 Compose + 临时恢复/升级演练替代 | 验证策略 | 需要永久 staging → 新 ADR 评估成本 |

### 15.3 条件化方案（无留白）

1. **同站无法满足** → 采用同源 BFF 或同源反代：浏览器只与同源 BFF 通信，refresh token 保存在 BFF 服务端；不把 `SameSite=None` 第三方 Cookie 作为生产主方案。触发：上线前域名/DNS 检查失败或 Safari/Chrome 无痕 E2E 失败。
2. **orval 8.46 在 Bun 1.4.2 下不可用** → 回退 `openapi-typescript` + `openapi-fetch`；保留同一 `ApiError`、单飞刷新、Bearer 注入与生成物 CI 漂移检查；不手写业务客户端。
3. **Argon2 生产基准 >300 ms** → 按 `p=1` → `m=32 MiB` 回退；不低于 `m=19 MiB/t=2/p=1`；在部署记录中写明最终参数与实测耗时。
4. **SQLx 0.9 与 Rust 1.94 不可用** → 暂时降级 SQLx 0.8.6；记录 ADR 修订；不改变 API/数据契约。
5. **HSTS preload 域名未确认** → 先只发 `max-age=31536000; includeSubDomains`，不加 `preload`；域名确认且所有子域 HTTPS 后追加。
6. **外部数据库不可用或 SLA 不满足** → 使用本地 `postgres:18.6-alpine` + pgBackRest；外部数据库只记录最低保证，迁移/备份责任随 `DATABASE_URL` 切换。
7. **实际用户/并发超过容量假设** → 先扩容主机/数据库；仍不足则按 §10.11 扩 API 副本，再评估托管 PG/多主机；触发条件见 §10.11。
8. **审计写入成为瓶颈** → 评估批量/异步锚定或外部存储；在改变链语义前必须新增 ADR；v1 保持同事务 fail-closed。
9. **合规要求外部 WORM** → 引入对象存储 Object Lock/日志外发；新增 ADR 与运维 runbook；不改变现有审计链。
10. **需要用户删除/批量操作/审计导出** → 属于新需求，新增 ADR 与 API 版本；不在 v1 静默加入。
11. **需要 RTL 语言** → 启用 locale→dir 映射与手动切换（或自动），补齐物理 CSS 检查；当前保留基础设施。
12. **需要邮件服务** → 重新评估邀请/重置链接，移除管理员直接设密；新增 ADR。
13. **密码 blocklist 来源许可证不兼容** → 换用等价可分发来源；保持“离线、版本化、前 100k、NFC+小写精确匹配”的契约。
14. **浏览器拒绝 localhost 的 Secure Cookie** → 使用 mkcert/HTTPS 本地开发；不移除 `Secure`/`__Host-`；README 记录。
15. **生产事故/主机故障** → 按 runbook 从异地备份恢复；RTO ≤2 小时为目标；事故后评估第二主机/托管 PG。

---

## 16. 参考与附录

### 16.1 ADR 索引

- [ADR-0001: 认证令牌与会话模型](../adr/0001-auth-token-session-model.md)
- [ADR-0002: RBAC 权限模型与授权规则](../adr/0002-rbac-permission-model.md)
- [ADR-0003: 用户生命周期与凭据管理](../adr/0003-user-lifecycle-and-credential-management.md)
- [ADR-0004: API 契约与数据模型](../adr/0004-api-and-data-contract.md)
- [ADR-0005: 安全基线](../adr/0005-security-baseline.md)
- [ADR-0006: 前端认证集成与路由守卫](../adr/0006-frontend-auth-integration.md)
- [ADR-0007: 前端裁剪、i18n 与 bun](../adr/0007-frontend-pruning-and-i18n.md)
- [ADR-0008: 部署、可观测与 CI](../adr/0008-deployment-observability-ci.md)

### 16.2 决策票据

- [生产可用登录系统实施规格（地图）](https://github.com/nasa-du/rs-admin-ds/issues/1)
- [Axum/SQLx/PostgreSQL 18 技术选型调研](https://github.com/nasa-du/rs-admin-ds/issues/2)
- [认证安全最佳实践调研](https://github.com/nasa-du/rs-admin-ds/issues/3)
- [shadcn-admin 集成审计](https://github.com/nasa-du/rs-admin-ds/issues/4)
- [跨域认证与令牌/会话模型](https://github.com/nasa-du/rs-admin-ds/issues/5)
- [RBAC 权限模型与授权规则](https://github.com/nasa-du/rs-admin-ds/issues/6)
- [账号生命周期与登录流程](https://github.com/nasa-du/rs-admin-ds/issues/7)
- [API 与数据契约](https://github.com/nasa-du/rs-admin-ds/issues/8)
- [安全基线：密码、限流、审计事件](https://github.com/nasa-du/rs-admin-ds/issues/9)
- [角色管理 API 与 UI](https://github.com/nasa-du/rs-admin-ds/issues/10)
- [审计日志查询 UI](https://github.com/nasa-du/rs-admin-ds/issues/11)
- [前端认证集成与路由守卫](https://github.com/nasa-du/rs-admin-ds/issues/12)
- [前端裁剪与中英 i18n](https://github.com/nasa-du/rs-admin-ds/issues/13)
- [Users 页面真实 CRUD 集成](https://github.com/nasa-du/rs-admin-ds/issues/14)
- [部署、可观测与 CI（含 PG18 升级预留、容量/SLO）](https://github.com/nasa-du/rs-admin-ds/issues/15)
- [规格整合与交付](https://github.com/nasa-du/rs-admin-ds/issues/16)

### 16.3 研究文档

- `docs/research/axum-stack.md`：Axum/SQLx/PostgreSQL 18 版本、离线模式、迁移、连接池、健康检查、优雅停机、PG18 兼容性。
- `docs/research/auth-security.md`：同站子域/BFF、令牌/会话、Argon2id、step-up、审计、密钥轮换、日志脱敏。
- `docs/research/shadcn-admin-audit.md`：上游快照、路由/页面、mock auth、Users/Settings、i18n/RTL、许可证。

### 16.4 外部标准与文档

- RFC 7807（Problem Details）、RFC 3339（时间）、RFC 9562（UUIDv7）、RFC 9106（Argon2）、RFC 8785（JCS）、RFC 6750（Bearer Token）、RFC 6265bis（Cookies）、RFC 9700（OAuth 安全最佳实践）。
- OpenAPI 3.1；NIST SP 800-63B-4；OWASP ASVS 5.0；OWASP Password Storage / Session Management / CSRF Cheat Sheets。
- PostgreSQL 18：`uuidv7()`、`pg_trgm`、`pg_upgrade`、`ssl`、`archive_command`。
- pgBackRest：stanza、WAL 归档、保留策略、`pgbackrest check`。
- nginx `limit_req`、certbot ACME HTTP-01、Prometheus/Alertmanager、GitHub Actions。

### 16.5 实现 PR 检查清单

每个实现 PR 必须：

- [ ] 引用本规格章节与至少一个 §14 验收 ID。
- [ ] 更新/新增测试；不降低既有测试覆盖。
- [ ] 若新增错误码/事件/权限，同时更新 OpenAPI、前端映射、审计目录与本文档。
- [ ] 若引入不可逆决策，新增/修订 ADR 并在 PR 描述中链接。
- [ ] 运行后端 fmt/clippy/test/sqlx prepare、前端 lint/typecheck/test/build、Compose config。
- [ ] 不提交 secret、真实用户数据、本地路径或调试凭据。
- [ ] 更新 `CONTEXT.md`（若新增领域术语）。
