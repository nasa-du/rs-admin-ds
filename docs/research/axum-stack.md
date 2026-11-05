# Axum / SQLx / PostgreSQL 18 技术选型调研

> 调研日期：2026-11-05
>
> 目标：为 `rs-admin-ds` 的生产可用登录系统确定 Rust 后端技术栈。前端为 `satnaing/shadcn-admin` 的裁剪 fork（React 19 + TanStack Query + TanStack Router + axios），后端采用 Axum + SQLx，数据库为 PostgreSQL 18，授权方式为 JWT。
>
> 方法：只使用 primary sources：crates.io、docs.rs、GitHub 源码/tag、npm registry、PostgreSQL 官方文档、RFC。所有版本号均在 2026-11-05 重新核验；版本会继续演进，落地前应再跑一次 `cargo search`/`cargo update`。
> 说明：本文只做选型与方案调研，不包含生产代码；代码片段仅用于说明 API 形状。

---

## 1. 结论摘要

### 1.1 推荐组合

| 关注点 | 推荐 crate / 工具 | 版本 | 备注 |
|---|---|---|---|
| HTTP framework | `axum` | `0.8.9` | 当前 stable；路由参数语法为 `/{id}`、`/{*rest}` |
| Async runtime | `tokio` | `1.54.1` | Axum/SQLx 均以 Tokio 为主 |
| Middleware | `tower` + `tower-http` | `0.5.3` / `0.7.1` | `trace`、`request-id`、`timeout`、`cors` 等 feature 默认关闭 |
| Database driver | `sqlx` | `0.9.0` | 编译期检查 SQL；MSRV `1.94.0` |
| Database | PostgreSQL | `18` | 使用 `18-alpine` 系列镜像（具体 patch tag 以 Docker Hub 为准） |
| Password hashing | `argon2` | `0.6.0` | Argon2id，PHC 字符串 |
| JWT | `jsonwebtoken` | `11.1.0` | 选 `rust_crypto` backend；EdDSA/RS256/HS256 均支持 |
| OpenAPI | `utoipa` + `utoipa-axum` | `6.0.0` / `0.3.0` | OpenAPI 3.1，axum 0.8 集成 |
| Swagger UI（可选） | `utoipa-swagger-ui` | `10.0.2` | 仅开发/内网启用 |
| TS client | `orval` | `8.46.0` | 生成 TanStack Query + axios；备选 `openapi-typescript` 7.13.0 |
| Rate limiting | `tower-governor` | `0.8.0` | 进程内 GCRA；无 Redis 时的首选 |
| Structured logging | `tracing` + `tracing-subscriber` | `0.1.44` / `0.3.23` | JSON + `EnvFilter` |
| Metrics | `metrics` + `metrics-exporter-prometheus` | `0.24.6` / `0.18.3` | Prometheus exposition |
| Integration testing | `testcontainers` + `testcontainers-modules` | `0.27.3` / `0.15.0` | 注意 modules 0.15 只兼容 core 0.27.x |
| Toolchain | Rust | `1.94.0+` | 由 SQLx 0.9 决定；edition 2024 可用 |

### 1.2 关键结论

1. **后端先按单实例设计，但不要在代码里依赖“只有单实例”。** `tower-governor` 的限流状态在进程内，多副本时每个副本各自计数，实际总限额是 `副本数 × 单副本限额`。无 Redis 时，跨副本的全局限额要么放到 ingress/WAF，要么用 PostgreSQL 原子计数，二者都要接受额外复杂度（见 §7）。
2. **SQLx 0.9 的 MSRV 是 1.94.0，是整个工作区 MSRV 的实际决定者。** Axum 0.8 只需要 1.80，`utoipa`/`jsonwebtoken`/`testcontainers` 需要 1.88，`argon2` 0.6 需要 1.85；取最大值即 1.94.0（见 §2）。
3. **JWT 建议默认 EdDSA（Ed25519），HS256 可以作为单服务 MVP 的简化选项。** 如果只有本服务签发、本服务校验，HS256 足够且最简单；但要满足“以后加服务/轮换密钥”的诉求，EdDSA 的公钥分发和 `kid` 轮换更干净（见 §4）。
4. **密码哈希不要直接依赖 `Argon2::default()` 的 19 MiB 参数作为长期策略。** RFC 9106 的第二推荐值是 64 MiB、t=3、p=4；但每个并发登录都会占用一份内存，必须同时做并发上限与登录限流（见 §5）。
5. **OpenAPI 3.1 + orval 8 最适合当前前端。** 前端已经是 TanStack Query 5 + axios，orval 的 `client: 'react-query'` + custom axios mutator 可以直接生成 hooks 和鉴权注入；如果 bun 对 orval 的 Node >=22.18 要求有兼容问题，退到 `openapi-typescript` + `openapi-fetch`（见 §6）。
6. **PostgreSQL 18 本身与 SQLx 兼容，但有两个明确的坑：** SQLx 仍发送 wire protocol 3.0（PG18 向后兼容 3.0），以及 PG18 新增的 `oauth` HBA method 不被 `sqlx-postgres` 支持（open issue #4399）。生产使用 `scram-sha-256`，不要用 `oauth` HBA（见 §10）。
7. **`uuidv7()` 是 PG18 新增能力，适合做时间有序主键。** 但它是 PG18-only；如果要求 schema 能回退到 PG17，改在 Rust 侧用 `Uuid::now_v7()` 生成。UUIDv7 只保证毫秒级时间有序，不应当作严格单调序列使用（见 §10.2）。

---

## 2. 版本与兼容性

### 2.1 核心版本矩阵

以下版本均取自 crates.io 的 `max_stable_version` 或对应 docs.rs/GitHub tag；npm 包取自 npm registry。

| Crate / 工具 | 最新 stable | MSRV / 运行时要求 | 来源 |
|---|---:|---|---|
| `axum` | 0.8.9 | Rust 1.80 | [crates.io](https://crates.io/crates/axum/0.8.9) |
| `tokio` | 1.54.1 | — | [crates.io](https://crates.io/crates/tokio/1.54.1) |
| `tower` | 0.5.3 | — | [crates.io](https://crates.io/crates/tower/0.5.3) |
| `tower-http` | 0.7.1 | crates.io 元数据为 Rust 1.65 | [crates.io](https://crates.io/crates/tower-http/0.7.1) |
| `sqlx` | 0.9.0 | Rust 1.94.0 | [changelog](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md) |
| `jsonwebtoken` | 11.1.0 | Rust 1.88.0 | [Cargo.toml](https://github.com/Keats/jsonwebtoken/blob/v11.1.0/Cargo.toml) |
| `josekit` | 0.10.3 | OpenSSL 1.1.1+ | [crates.io](https://crates.io/crates/josekit/0.10.3) |
| `argon2` | 0.6.0 | Rust 1.85 | [changelog](https://github.com/RustCrypto/password-hashes/blob/argon2-v0.6.0/argon2/CHANGELOG.md) |
| `utoipa` | 6.0.0 | Rust 1.88 | [crates.io](https://crates.io/crates/utoipa/6.0.0) |
| `utoipa-axum` | 0.3.0 | 依赖 `axum ^0.8.4`、`utoipa ^6.0.0` | [docs.rs](https://docs.rs/utoipa-axum/0.3.0/utoipa_axum/) |
| `utoipa-swagger-ui` | 10.0.2 | 依赖 `utoipa ^6.0.0` | [crates.io](https://crates.io/crates/utoipa-swagger-ui/10.0.2) |
| `aide`（备选） | 0.15.1 | 依赖 `axum ^0.8.1`、`schemars ^0.9` | [docs.rs](https://docs.rs/aide/0.15.1/aide/) |
| `tower-governor` | 0.8.0 | 依赖 `governor ^0.10.0` | [crates.io](https://crates.io/crates/tower-governor/0.8.0) |
| `governor` | 0.10.4 | — | [crates.io](https://crates.io/crates/governor/0.10.4) |
| `tracing` | 0.1.44 | — | [crates.io](https://crates.io/crates/tracing/0.1.44) |
| `tracing-subscriber` | 0.3.23 | — | [crates.io](https://crates.io/crates/tracing-subscriber/0.3.23) |
| `metrics` | 0.24.6 | — | [crates.io](https://crates.io/crates/metrics/0.24.6) |
| `metrics-exporter-prometheus` | 0.18.3 | 依赖 `metrics ^0.24.5` | [crates.io](https://crates.io/crates/metrics-exporter-prometheus/0.18.3) |
| `testcontainers` | 0.27.3（0.27.x 最新）；0.28.0 已发布 | Rust 1.88 | [crates.io](https://crates.io/crates/testcontainers/0.27.3) |
| `testcontainers-modules` | 0.15.0 | 依赖 `testcontainers ^0.27.0` | [Cargo.toml](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/Cargo.toml) |
| `uuid` | 1.29.0 | — | [crates.io](https://crates.io/crates/uuid/1.29.0) |
| `orval` (npm) | 8.46.0 | Node >= 22.18.0 | [npm](https://www.npmjs.com/package/orval/v/8.46.0) |
| `openapi-typescript` (npm) | 7.13.0 | npm 包未声明 `engines` | [npm](https://www.npmjs.com/package/openapi-typescript/v/7.13.0) |
| `openapi-fetch` (npm) | 0.17.0 | — | [npm](https://www.npmjs.com/package/openapi-fetch/v/0.17.0) |

### 2.2 MSRV 与 edition

- SQLx 0.9.0 的 changelog 明确写明本 release cycle 支持 Rust `1.94.0`，这是当前工作区 MSRV 的下限；因此建议在 `Cargo.toml`/`rust-toolchain.toml` 里固定 `rust-version = "1.94"`，CI 使用当前 stable（≥1.94）。来源：[SQLx 0.9.0 changelog](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md)。
- Axum 0.8 的 MSRV 是 1.80；`jsonwebtoken` 11.1.0 使用 edition 2024 且 MSRV 1.88；`utoipa` 6 的 README 标注 MSRV 1.88；`argon2` 0.6 的 MSRV 1.85；`testcontainers` 0.27.3 的 MSRV 1.88。取最大值后仍是 SQLx 的 1.94。
- 如果短期内无法升级到 1.94，需要把 SQLx 降到 0.8.x（0.8.6 仍可用），但会失去 0.9 的 `sqlx.toml` 等改进；这是降级方案，不是首选。来源：[SQLx changelog](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md)。

### 2.3 Axum 0.8 注意事项

- Axum 0.8 将 path parameter 语法从 `/:id`、`/*rest` 改为 `/{id}`、`/{*rest}`；旧语法会 panic。来源：[axum 0.8.0 changelog](https://github.com/tokio-rs/axum/blob/main/axum/CHANGELOG.md)。
- `axum::serve(listener, app)` 是 0.8 的入口；需要 `ConnectInfo<SocketAddr>` 时使用 `app.into_make_service_with_connect_info::<SocketAddr>()`，这也是 `tower-governor` 的 `PeerIpKeyExtractor` 正常工作的前提。来源：[axum `serve`](https://docs.rs/axum/0.8.9/axum/fn.serve.html)、[tower-governor README](https://github.com/benwis/tower-governor#common-pitfalls)。
- `tower-http` 0.7.1 依赖 `http 1.x` / `tower 0.5.x`，与 Axum 0.8 兼容；但它的 feature 默认是空的，`TraceLayer`、`SetRequestIdLayer` 等都需要显式开启 `trace`、`request-id`。来源：[crates.io tower-http](https://crates.io/crates/tower-http/0.7.1)、[TraceLayer docs](https://docs.rs/tower-http/0.7.1/tower_http/trace/struct.TraceLayer.html)。

建议的依赖形态（片段）：

```toml
[dependencies]
axum = "0.8.9"
tokio = { version = "1.54.1", features = ["full"] }
tower = "0.5.3"
tower-http = { version = "0.7.1", features = ["trace", "request-id", "timeout", "cors"] }
sqlx = { version = "0.9.0", default-features = false, features = [
    "runtime-tokio",
    "tls-rustls-ring",
    "postgres",
    "macros",
    "migrate",
    "uuid",
    "chrono",
    "json",
] }
```

SQLx 默认 feature 是 `any`、`macros`、`migrate`、`json`，**不包含 runtime 和 driver**；`postgres`、`runtime-tokio`、TLS 必须显式添加。TLS feature 的取舍见 §3.1。来源：[crates.io sqlx 0.9.0 features](https://crates.io/crates/sqlx/0.9.0)。

---

## 3. SQLx 0.9：编译期查询、offline mode 与迁移

### 3.1 依赖与 feature 选择

SQLx 0.9.0 是当前 stable，仓库已从 `launchbadge/sqlx` 迁移到 `transact-rs/sqlx`；0.9 的 SQLx 仓库本身不再跟踪 `Cargo.lock`，所以 `cargo install --locked sqlx-cli` 不再可用；应用项目仍应提交自己的 `Cargo.lock`。来源：[SQLx 0.9.0 changelog](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md)。

推荐的 feature：

- `runtime-tokio`：Tokio runtime。
- `postgres`：PostgreSQL driver。
- `macros`：`query!`/`query_as!` 等宏；该 feature 同时启用 offline 相关依赖。
- `migrate`：`sqlx::migrate!()`、`Migrator`、`sqlx migrate`。
- `uuid`、`chrono`（或 `time`）：类型映射。
- TLS：`tls-rustls-ring`（ring + webpki roots）、`tls-rustls-aws-lc-rs`、`tls-native-tls`，或本地无 TLS 时 `tls-none`。生产连接托管 PostgreSQL 时建议 `tls-rustls-ring` 或 `tls-rustls-aws-lc-rs`，并配合 `sslmode=verify-full`。

来源：[crates.io sqlx 0.9.0 features](https://crates.io/crates/sqlx/0.9.0)、[SQLx README](https://github.com/transact-rs/sqlx/blob/v0.9.0/README.md)。

### 3.2 编译期查询与 offline mode（`.sqlx`）

`query!`/`query_as!` 在编译期需要知道数据库 schema，有两种模式：

1. **在线模式**：编译时设置 `DATABASE_URL`，SQLx 连接数据库并 prepare/describe 查询。
2. **offline mode**：把查询元数据保存到 `.sqlx` 目录并提交到版本控制，编译时不再需要数据库。

来源：[`query!` macro docs](https://docs.rs/sqlx/0.9.0/sqlx/macro.query.html)。

标准工作流：

```bash
# 1) 本地/CI 有一个已应用迁移的 PostgreSQL
export DATABASE_URL='postgres://user:pass@localhost:5432/app'

# 2) 生成元数据（工作区根目录）
cargo sqlx prepare --workspace

# 3) 将 .sqlx 提交到版本控制
git add .sqlx

# 4) CI 校验 .sqlx 是否与当前 schema/queries 一致；不一致时退出码非 0
cargo sqlx prepare --check --workspace
```

关键细节（均来自 [sqlx-cli README](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-cli/README.md)）：

- 单 crate 项目用 `cargo sqlx prepare`，元数据写在当前目录的 `.sqlx`；workspace 用 `--workspace`，在 workspace 根目录生成一个 `.sqlx`。
- `.sqlx` 应提交到版本控制；提交后本地构建不再需要数据库。
- `cargo sqlx prepare --check` 在 `.sqlx` 过期时以非 0 退出，专用于 CI。
- `DATABASE_URL` 的优先级高于 `.sqlx`；如果 CI 环境里意外存在 `DATABASE_URL`，构建会尝试连库。用 `SQLX_OFFLINE=true` 强制 offline。
- 测试或 feature-gated 的查询需要显式打开 target/feature，否则 `prepare` 看不到它们：
  ```bash
  cargo sqlx prepare --workspace -- --all-targets --all-features
  ```
- `SQLX_OFFLINE_DIR` 可覆盖 `.sqlx` 的位置（0.9 支持）；默认优先使用环境变量。来源：[SQLx 0.9 changelog #3962](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md)。

CI 的两个容易踩的点：

1. **`prepare --check` 任务需要真实数据库。** 它要对比当前 schema 与 `.sqlx`，所以该 job 必须启动 PostgreSQL 18、执行 `sqlx migrate run`，再跑 `cargo sqlx prepare --check --workspace -- --all-targets --all-features`。
2. **测试 job 可以同时设置 `DATABASE_URL` 和 `SQLX_OFFLINE=true`。** `SQLX_OFFLINE=true` 让编译期宏使用 `.sqlx`；运行期 `#[sqlx::test]`/连接池仍可用 `DATABASE_URL` 连测试库。这样既验证离线元数据，又跑真实数据库测试。

sqlx-cli 版本要与 `sqlx` 的 major/minor 对齐。0.9 的 changelog 明确说明 `cargo install --locked sqlx-cli` 不再可用（`Cargo.lock` 已移除），安装示例：

```bash
cargo install sqlx-cli --version 0.9.0 --no-default-features --features rustls,postgres
# 如果使用 sqlx.toml，再启用 sqlx-toml feature：
# --features rustls,postgres,sqlx-toml
```

来源：[SQLx 0.9.0 changelog](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md)、[sqlx-cli feature 列表](https://crates.io/crates/sqlx-cli/0.9.0)。

### 3.3 迁移工具（migrations）

SQLx 自带 migration 工具，不依赖 Diesel/Refinery。

常用命令（来自 [sqlx-cli README](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-cli/README.md)）：

```bash
sqlx migrate add <name>          # 创建 migrations/<timestamp>-<name>.sql
sqlx migrate add -r <name>       # 创建 .up.sql/.down.sql 可回滚迁移
sqlx migrate run                 # 应用尚未执行的迁移
sqlx migrate revert              # 回滚最近一个 down migration
sqlx migrate info --source ../relative/migrations
```

应用内嵌迁移：

```rust
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!(); // 默认 ./migrations
```

- `sqlx::migrate!()` 的路径相对 `Cargo.toml` 所在目录，而不是调用宏的源文件；默认是 `./migrations`。来源：[`migrate!` docs](https://docs.rs/sqlx/0.9.0/sqlx/macro.migrate.html)。
- 迁移文件的行尾会影响 checksum。跨平台仓库应加 `.gitattributes`：
  ```gitattributes
  *.sql text eol=lf
  ```
  或在 `sqlx.toml` 中配置忽略 whitespace。来源：[`migrate!` docs](https://docs.rs/sqlx/0.9.0/sqlx/macro.migrate.html)。
- 修改已应用的迁移会触发 checksum mismatch；SQLx 的 `Migration` 使用 SHA-384 计算 checksum，`Migrator` 默认 `_sqlx_migrations` 表、`locking = true`、`ignore_missing = false`。不要编辑已应用的 migration，新增一个 migration。来源：[`Migrator` docs](https://docs.rs/sqlx/0.9.0/sqlx/migrate/struct.Migrator.html)、[`migration.rs`](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/migrate/migration.rs)、[`migrator.rs`](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/migrate/migrator.rs)。
- `Migrator::run()` 默认在 PostgreSQL 上获取 advisory lock，多个副本同时启动时不会并发执行同一批迁移；但生产仍建议把 migration 作为独立部署步骤，使用单独的 migration role，应用 runtime role 不授予 DDL 权限。来源：[`Migrator` docs](https://docs.rs/sqlx/0.9.0/sqlx/migrate/struct.Migrator.html)。
- 如果使用 `sqlx::migrate!()` 嵌入迁移，新增 migration 文件不会自动触发 Rust 重新编译；需要 `build.rs`：
  ```rust
  fn main() {
      println!("cargo:rerun-if-changed=migrations");
  }
  ```
  也可用 `sqlx migrate build-script` 生成。来源：[`migrate!` docs](https://docs.rs/sqlx/0.9.0/sqlx/macro.migrate.html)。

### 3.4 连接池

`PgPoolOptions::new()` 的默认值是（源码）：`max_connections = 10`、`min_connections = 0`、`acquire_timeout = 30s`、`idle_timeout = 10min`、`max_lifetime = 30min`、`test_before_acquire = true`。来源：[`pool/options.rs`](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/pool/options.rs)。

建议：

- 显式设置 `max_connections`，并与 PostgreSQL 的 `max_connections`、副本数一起计算：`副本数 × pool_max + 迁移/运维连接 < 数据库上限`。
- 设置 `min_connections` 以降低冷启动延迟，但不要等于 max，避免空闲占用。
- `acquire_timeout` 要小于请求超时，避免请求排队到超过上游 deadline。
- 关闭时调用 `pool.close().await`，等待 checked-out 连接归还；`close()` 会让后续 `acquire()` 立即返回 `PoolClosed`。来源：[`Pool::close` docs](https://docs.rs/sqlx-core/0.9.0/sqlx_core/pool/struct.Pool.html#method.close)。

### 3.5 `#[sqlx::test]`

`#[sqlx::test]` 为每个测试创建独立数据库，并自动应用 `CARGO_MANIFEST_DIR/migrations`（除非显式关闭）。来源：[`#[sqlx::test]` docs](https://docs.rs/sqlx/0.9.0/sqlx/attr.test.html)。

要点：

- PostgreSQL 必须设置 `DATABASE_URL`，且连接用户需要有创建/删除测试数据库的权限（通常是 superuser）。
- 测试成功后自动清理数据库；失败的测试会保留数据库便于排查，但下一次运行同一测试二进制时会清理旧库。
- 支持 fixtures：`#[sqlx::test(fixtures("users"))]`。
- 可以用 `migrator = "crate::MIGRATOR"` 复用应用内嵌的 `Migrator`。
- 也可以注入 `PgPool`、`PoolConnection<Postgres>`、`PoolOptions<Postgres>` 等参数。

### 3.6 SQLx 0.9 的注意点

- 0.9.0 于 2026-05-06 发布，仓库迁移到 `transact-rs`；生态里的辅助 crate 可能还没完全跟上。来源：[changelog](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md)。
- 0.9 引入 `sqlx.toml`（迁移表重命名、schema、忽略 whitespace 等）；使用 CLI 时记得启用 `sqlx-toml` feature，否则 CLI 可能不读取配置。来源：[`migrate!` docs](https://docs.rs/sqlx/0.9.0/sqlx/macro.migrate.html)、[sqlx-cli features](https://crates.io/crates/sqlx-cli/0.9.0)。
- 0.9 对 Postgres 的 nullability 推断有调整：changelog #3541 改为强制 generic plan，以改善 nullability 推断。如果表达式列的推断与预期不符，可先用 `AS "col!"` 显式标注非空，但先确认它不是真实 nullable，不要滥用。来源：[changelog](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md)。

---

## 4. JWT：库对比、算法选择、轮换与校验

### 4.1 `jsonwebtoken` vs `josekit`

| 维度 | `jsonwebtoken` 11.1.0 | `josekit` 0.10.3 |
|---|---|---|
| 维护状态 | 2026-09-16 发布 11.1.0；Cargo.toml 标注 `passively-maintained` | 2025-05-20 发布 0.10.3，更新较慢 |
| Crypto backend | `rust_crypto`（纯 Rust）或 `aws_lc_rs`；二者最多选一个 | 依赖 OpenSSL 1.1.1+ |
| 算法 | HS256/384/512、RS256/384/512、PS256/384/512、ES256/384、EdDSA | HS/RS/PS/ES（含 ES256K）、EdDSA（Ed25519/Ed448）、JWE、`none` |
| JWK | `Jwk`、`JwkSet`、`JwkSet::find(kid)`、`Jwk::thumbprint` | `Jwk`、`JwkSet`、按 JWK set 校验 |
| HTTP JWKS 获取/缓存 | 不提供，需要应用自己实现 | 不提供（库只处理 JOSE/JWK 数据结构） |
| 适用场景 | 自签自验的 access token，纯 Rust/静态链接友好 | 需要 JWE、Ed448、OpenSSL 生态或已有 OpenSSL 部署 |

来源：[jsonwebtoken README](https://github.com/Keats/jsonwebtoken/blob/v11.1.0/README.md)、[jsonwebtoken Cargo.toml](https://github.com/Keats/jsonwebtoken/blob/v11.1.0/Cargo.toml)、[josekit README](https://github.com/jedisct1/rust-josekit/blob/master/README.md)、[josekit docs](https://docs.rs/josekit/0.10.3/josekit/)。

**推荐：`jsonwebtoken = { version = "11.1.0", features = ["rust_crypto"] }`。**

- `rust_crypto` 是纯 Rust，交叉编译/静态链接（musl、distroless）比 OpenSSL 简单。
- 需要更高签名吞吐或 FIPS 相关能力时，可换 `aws_lc_rs`；但二者只能选一个。来源：[jsonwebtoken README](https://github.com/Keats/jsonwebtoken/blob/v11.1.0/README.md)。
- 如果不需要 PEM 解析，可以 `default-features = false, features = ["rust_crypto"]` 去掉 `use_pem`；但通常保留默认的 `use_pem` 更方便加载 PEM 私钥。

### 4.2 HS256 vs EdDSA vs RS256

| 维度 | HS256 | EdDSA（Ed25519） | RS256 |
|---|---|---|---|
| 密钥 | 共享 secret | 私钥签名 / 公钥验签 | RSA 私钥签名 / 公钥验签 |
| 签名大小 | 32 bytes | 64 bytes | 256 bytes（RSA-2048） |
| 验签成本 | 很低 | 很低 | 较高 |
| 密钥分发 | 每个验签方都要拿到 secret | 只需分发公钥/JWKS | 只需分发公钥/JWKS |
| 轮换 | 多 secret + `kid`；旧 secret 需保留到 token 过期 | 新私钥 + 新 `kid`；旧公钥保留到 token 过期 | 同 EdDSA |
| 主要风险 | secret 泄漏即可伪造；secret 太短 | 实现/曲线支持差异；`alg` 必须固定 | RSA 配置错误、填充模式混淆；`alg` 必须固定 |
| 适用 | 单服务、自签自验、MVP | 新系统默认；未来多服务 | 已有 RSA/JWKS 基础设施、外部 IdP |

来源：[RFC 7518 §3.2](https://www.rfc-editor.org/rfc/rfc7518.html#section-3.2)（HS256 的 key 至少应与 hash 输出等长，即 256 bits）、[RFC 8037](https://www.rfc-editor.org/rfc/rfc8037.html)（EdDSA）、[RFC 8725](https://www.rfc-editor.org/rfc/rfc8725.html)。

建议：

- **默认选 EdDSA（Ed25519）**，`Header::new(Algorithm::EdDSA)`，私钥用 PKCS#8 PEM，公钥发布为 JWKS。`jsonwebtoken` 支持 `EncodingKey::from_ed_pem` / `DecodingKey::from_ed_pem` / `DecodingKey::from_jwk`。来源：[`EncodingKey` docs](https://docs.rs/jsonwebtoken/11.1.0/jsonwebtoken/struct.EncodingKey.html)、[`DecodingKey` docs](https://docs.rs/jsonwebtoken/11.1.0/jsonwebtoken/struct.DecodingKey.html)。
- **如果只有本服务签发和校验，且希望最小运维面**，HS256 可以接受；secret 必须是 CSPRNG 生成的 ≥32 bytes，不得使用人类口令，且要设计双 secret 轮换。
- **RS256 仅在需要与现有 RSA/JWKS 生态互操作时选择**；不要因为“JWT 默认示例都是 RS256”而选它。
- 前端永远不持有签名私钥/secret；前端只保存并发送 token（更推荐 httpOnly cookie，见安全专项调研）。JWT 不是加密，payload 可被读取，不要放敏感信息。来源：[RFC 7519 §4.1](https://www.rfc-editor.org/rfc/rfc7519.html#section-4.1)。

### 4.3 推荐的最小实现形状

```rust
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};

#[derive(serde::Serialize, serde::Deserialize)]
struct Claims {
    sub: String,      // user UUID
    jti: String,      // session / token id
    iss: String,
    aud: String,
    exp: usize,       // seconds since UNIX epoch
    iat: usize,
    token_use: String, // "access"
}

let mut header = Header::new(Algorithm::EdDSA);
header.kid = Some(active_kid.clone());
let token = encode(&header, &claims, &EncodingKey::from_ed_pem(private_pem)?)?;

let header = jsonwebtoken::decode_header(&token)?;
let kid = header.kid.ok_or(AuthError::MissingKid)?;
let jwk = jwks.find(&kid).ok_or(AuthError::UnknownKid)?; // jwks 来自本地可信配置
let key = DecodingKey::from_jwk(jwk)?;

let mut validation = Validation::new(Algorithm::EdDSA);
validation.set_audience(&["rs-admin-web"]);
validation.set_issuer(&["rs-admin-api"]);
validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
validation.leeway = 5; // 默认 60，按部署环境调整

let data = decode::<Claims>(&token, &key, &validation)?;
assert_eq!(data.claims.token_use, "access");
```

来源：[jsonwebtoken README](https://github.com/Keats/jsonwebtoken/blob/v11.1.0/README.md)、[`Validation` docs](https://docs.rs/jsonwebtoken/11.1.0/jsonwebtoken/struct.Validation.html)、[`JwkSet` docs](https://docs.rs/jsonwebtoken/11.1.0/jsonwebtoken/jwk/struct.JwkSet.html)。

### 4.4 密钥轮换与 JWKS

推荐流程：

1. 生成新密钥对，分配新的 `kid`；私钥写入 secret manager/KMS，公钥加入 JWKS。
2. 新签发的 token 使用新 `kid` 签名；旧私钥停止签名。
3. 验签端保留**旧公钥 + 新公钥**，直到旧 token 的最大 `exp` 已过（再加 clock skew/leeway）。
4. 旧公钥从 JWKS 移除；旧 token 自然失效。
5. 如果使用 HS256，则维护 `kid -> secret` 的 map；旧 secret 同样保留到旧 token 过期。HS256 的“公钥”就是 secret，**绝不能发布任何 oct JWK**。

`jsonwebtoken` 的 JWK 支持：

- `Jwk::from_encoding_key(&encoding_key, Algorithm::EdDSA)` 可从私钥提取公钥组件并构造 JWK；设置 `jwk.common.key_id = Some(kid)` 后放入 `JwkSet`。来源：[`Jwk::from_encoding_key` docs](https://docs.rs/jsonwebtoken/11.1.0/jsonwebtoken/jwk/struct.Jwk.html#method.from_encoding_key)。
- 对 HS256，`Jwk::from_encoding_key` 会生成 `kty: "oct"` 并包含 secret 本身；该 JWK 只能留在服务端，不能出现在任何公开响应中。来源：[`jwk.rs`](https://github.com/Keats/jsonwebtoken/blob/v11.1.0/src/jwk.rs)。
- `JwkSet::find(kid)` 按 `kid` 查找；没有 `kid` 的 key 不会被匹配。来源：[`JwkSet::find` docs](https://docs.rs/jsonwebtoken/11.1.0/jsonwebtoken/jwk/struct.JwkSet.html#method.find)。
- `jsonwebtoken` **不提供** HTTP JWKS 客户端、缓存和刷新；如果只自签自验，直接从本地配置/secret 加载 JWKS，避免网络依赖；只有接外部 IdP 时才实现带 TTL 和 single-flight 的 JWKS 缓存，并对未知 `kid` 最多触发一次刷新。

安全边界：

- 只根据 `kid` 在**本地可信 key set**里选择 key；不要信任 token header 里的 `jku`、`x5u`、`jwk` 去远程取 key。RFC 8725 §3.10 明确要求不要信任这些 header。来源：[RFC 8725 §3.10](https://www.rfc-editor.org/rfc/rfc8725.html#section-3.10)。
- 轮换期间允许旧 `kid` 验签，但必须仍在本地 key set 内；未知 `kid` 直接拒绝，不要回退到“尝试所有 key”作为常规路径（可以做一次受控刷新）。
- 公钥 JWKS 是公开信息，但仍应通过 HTTPS 提供、设置缓存头，并限制刷新频率。

### 4.5 校验陷阱清单

1. **固定算法。** `Validation::new(Algorithm::EdDSA)` 会把 `algorithms` 设为 `[EdDSA]`；`decode()` 在验签前检查 `header.alg` 是否在 `validation.algorithms` 中，不在则返回 `InvalidAlgorithm`。来源：[`decoding.rs`](https://github.com/Keats/jsonwebtoken/blob/v11.1.0/src/decoding.rs)。
2. **不要使用 `insecure_decode`/`insecure_decode_claims`。** 它们明确“zero validation”，只能用于调试。来源：[`insecure_decode` docs](https://docs.rs/jsonwebtoken/11.1.0/jsonwebtoken/fn.insecure_decode.html)。
3. **不要接受 `alg: none`。** `jsonwebtoken` 不把 `none` 列为支持算法；仍然要在网关/代码层显式拒绝。
4. **`exp` 默认校验，`nbf` 默认不校验。** `Validation` 默认 `required_spec_claims = {"exp"}`、`validate_exp = true`、`validate_nbf = false`、`leeway = 60`。如果 token 使用 `nbf`，显式 `validate_nbf = true` 并把 `nbf` 加入 required claims。来源：[`Validation` docs](https://docs.rs/jsonwebtoken/11.1.0/jsonwebtoken/struct.Validation.html)。
5. **`aud` 的默认行为容易误解。** 默认 `validate_aud = true`，但 `aud = None`；此时如果 token 带 `aud`，会返回 `InvalidAudience`；如果 token 不带 `aud`，则不会报错（除非把 `aud` 设为 required）。所以必须 `set_audience(...)` 并把 `aud` 加入 required claims。来源：[`validation.rs`](https://github.com/Keats/jsonwebtoken/blob/v11.1.0/src/validation.rs)。
6. **`iss` 只有在配置且 token 包含该 claim 时才校验。** 如果 `validation.iss = None`，token 里的 `iss` 不会导致失败；如果 token 没有 `iss`，仅 `set_issuer(...)` 也不会失败，必须同时把 `iss` 加入 required claims。来源：[`Validation` docs](https://docs.rs/jsonwebtoken/11.1.0/jsonwebtoken/struct.Validation.html)。
7. **`iat` 不参与默认校验。** 不要用 `iat` 判断 token 新鲜度；用短 `exp` + `jti`/session 表。
8. **HMAC secret 长度。** RFC 7518 §3.2 要求 HMAC key 至少与 hash 输出等长（HS256 ≥ 256 bits）；不要从密码派生后直接当 secret。来源：[RFC 7518 §3.2](https://www.rfc-editor.org/rfc/rfc7518.html#section-3.2)。
9. **显式 token 类型。** 如果同时有 access/refresh 或多种 token，使用 `token_use` claim 或 `typ` header 并校验；不要只凭“能验签”就当作 access token。RFC 8725 §3.11 建议显式 typing。来源：[RFC 8725 §3.11](https://www.rfc-editor.org/rfc/rfc8725.html#section-3.11)。
10. **不要在日志里打印 token。** 见 §8.1；Authorization/Cookie/refresh token 都属于敏感数据。

### 4.6 Claims 设计建议

Access token（短生命周期）：

- `sub`：user UUID（字符串）。
- `jti`：session id 或 token id，便于撤销/审计。
- `iss`：`rs-admin-api`；`aud`：`rs-admin-web`。
- `exp`：10–15 分钟；`iat`：签发时间。
- `token_use`：`access`。
- `ver`（可选）：用户 token version；改密码/强制下线时递增，使旧 token 失效。
- 不建议把大量权限/角色塞进 token；权限变更后旧 token 在过期前仍带着旧权限。可以在每次请求查一次用户/角色，或用短 TTL + token version。

Refresh token：

- 推荐 opaque random token（≥256 bits），数据库只存 SHA-256 hash，支持 rotation、reuse detection 和按 session 撤销；不要把长期 refresh token 做成不可撤销的 JWT。
- 如果坚持 JWT refresh token，至少使用独立 `token_use = "refresh"`、独立 audience、较长 `exp`、`jti` 入库，并在每次刷新时轮换。

以上 claims 名称来自 RFC 7519 的 registered claims；`token_use` 是应用自定义 claim。来源：[RFC 7519 §4.1](https://www.rfc-editor.org/rfc/rfc7519.html#section-4.1)。

---

## 5. Argon2id

### 5.1 crate 与 API

推荐 `argon2 = "0.6.0"`（RustCrypto）。0.6.0 于 2026-08-27 发布，升级到 `password-hash 0.6` / `phc 0.6`，MSRV 1.85；API 与 0.5.x 有差异（`PasswordHash` 推荐路径变为 `password_hash::phc::PasswordHash`，`simple` feature 改名为 `password-hash`）。来源：[argon2 0.6.0 changelog](https://github.com/RustCrypto/password-hashes/blob/argon2-v0.6.0/argon2/CHANGELOG.md)。

- 默认算法是 Argon2id v19；`Argon2::default()` 使用 `m_cost = 19 * 1024`（19 MiB）、`t_cost = 2`、`p_cost = 1`、output 32 bytes。来源：[`params.rs`](https://github.com/RustCrypto/password-hashes/blob/argon2-v0.6.0/argon2/src/params.rs)、[`Params` docs](https://docs.rs/argon2/0.6.0/argon2/struct.Params.html)。
- `hash_password` 在启用 `getrandom`（默认）时自动生成随机 salt，输出 PHC 字符串（`$argon2id$v=19$m=...,t=...,p=...$salt$hash`）。来源：[`Argon2` docs](https://docs.rs/argon2/0.6.0/argon2/struct.Argon2.html)。
- `verify_password` 使用 PHC 字符串里保存的参数，而不是当前 `Argon2` 实例的参数；因此升级参数不会让旧 hash 失效。来源：[argon2 crate docs](https://docs.rs/argon2/0.6.0/argon2/)。

API 形状：

```rust
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

// 生产参数示例：64 MiB, t=3, p=4（按机器内存和并发调整）
let params = Params::new(64 * 1024, 3, 4, Some(32))?;
let argon2 = Argon2::new(Algorithm::Argon2id, Version::default(), params);

let stored = argon2.hash_password(password.as_bytes())?.to_string();

let parsed = PasswordHash::new(&stored)?;
argon2.verify_password(password.as_bytes(), &parsed)?;
```

- `Params::new(m_cost, t_cost, p_cost, output_len)` 的 `m_cost` 单位是 1 KiB block；64 MiB = `64 * 1024`。来源：[`Params::new` docs](https://docs.rs/argon2/0.6.0/argon2/struct.Params.html#method.new)。
- 启用 `parallel` feature 后，`p_cost > 1` 时使用 rayon 并行计算 lane；不启用时即使 `p_cost > 1` 也是单线程计算。来源：[`memory.rs`](https://github.com/RustCrypto/password-hashes/blob/argon2-v0.6.0/argon2/src/memory.rs)。
- 可选 pepper：`Argon2::new_with_secret(secret, ...)`；pepper 必须存于 KMS/secret manager，不能与 hash 同库。pepper 轮换会导致旧 hash 无法验证，需要支持多 pepper version 或强制重置密码。来源：[`Argon2::new_with_secret` docs](https://docs.rs/argon2/0.6.0/argon2/struct.Argon2.html#method.new_with_secret)。

### 5.2 参数选择

RFC 9106 §4 给出两个推荐：

- **FIRST RECOMMENDED**：Argon2id，`t=1`，`p=4`，`m=2^21`（2 GiB），128-bit salt，256-bit tag。
- **SECOND RECOMMENDED**：Argon2id，`t=3`，`p=4`，`m=2^16`（64 MiB），128-bit salt，256-bit tag。

来源：[RFC 9106 §4](https://www.rfc-editor.org/rfc/rfc9106.html#section-4)。

工程建议：

- 登录系统通常选 **64 MiB / t=3 / p=4**（RFC 第二推荐）作为目标；如果部署内存有限，`Argon2::default()` 的 19 MiB / t=2 / p=1 是 crate 默认值，但低于 RFC 第二推荐，只应作为最低线。
- 用生产硬件 benchmark 单次 hash 时间，目标约 100–300 ms；不要只照搬参数。
- **内存是并发的乘数**：64 MiB × 同时进行的 hash 数。100 个并发登录可能瞬时占用 6.25 GiB（6400 MiB）。必须用 `tokio::sync::Semaphore` 限制并发 hash 数（例如 CPU 核数），并在登录接口做严格限流；否则 Argon2 会成为内存/CPU DoS 放大器。
- `p=4` 且启用 `parallel` 时，每次 hash 可能占用 4 个线程；并发上限要同时考虑 CPU 和内存。
- 参数升级后，旧 hash 仍可验证；登录成功时比较 PHC 里的 `m/t/p` 与当前策略，若低于策略则用新参数 rehash 并更新数据库。可以解析 `parsed.params`，或在用户表加 `password_version` 列。

### 5.3 存储与校验细节

- 数据库列用 `TEXT`/`VARCHAR` 保存完整 PHC 字符串即可，不需要单独的 salt/params 列；PHC 字符串自带算法、版本、参数、salt、hash。来源：[password-hash 0.6 docs](https://docs.rs/password-hash/0.6.0/password_hash/)。
- 校验失败与未知用户的响应要保持一致，避免 user enumeration；未知用户也可以对一个固定的 dummy PHC hash 执行一次 verify，以降低时间差。该建议属于认证安全工程实践，具体策略可与安全专项调研合并。
- 不要截断密码；Argon2 没有 bcrypt 的 72-byte 限制，但应对超长输入设置合理上限（例如 1 KiB）并在 API 层拒绝异常大的请求体。
- 密码 hash 不可逆；不要试图“解密”或在日志中输出。

---

## 6. OpenAPI 生成与 TypeScript 客户端

### 6.1 utoipa 6 + utoipa-axum 0.3（推荐）

`utoipa` 6.0.0 支持 OpenAPI 3.1，默认启用 `macros`。来源：[utoipa README](https://github.com/juhaku/utoipa/blob/master/README.md)。

`utoipa-axum` 0.3.0 提供 `OpenApiRouter`、`routes!` 宏和 `split_for_parts()`，并明确依赖 `axum ^0.8.4`、`utoipa ^6.0.0`。来源：[utoipa-axum docs](https://docs.rs/utoipa-axum/0.3.0/utoipa_axum/)。

典型形状：

```rust
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(OpenApi)]
#[openapi(
    info(title = "rs-admin API", version = "0.1.0"),
    components(schemas(LoginRequest, TokenResponse, User))
)]
struct ApiDoc;

let (api_router, mut api) = OpenApiRouter::<AppState>::new()
    .routes(routes!(login, logout, me))
    .split_for_parts();
api.merge(ApiDoc::openapi());

let app = axum::Router::new()
    .nest("/api/v1", api_router)
    .route("/api-docs/openapi.json", axum::routing::get(|| async move {
        axum::Json(api)
    }));
```

Bearer security scheme 通过 utoipa 的 `SecurityScheme`/`security_schemes` 组件声明，并在需要鉴权的 handler 上标注 `security(...)`；具体语法以 utoipa 6 的 `#[openapi]` 文档为准。

- `OpenApi` 实现了 `Serialize`，也提供 `to_json()` / `to_pretty_json()` / `to_yaml()`；可以直接 `Json(api)` 或输出字符串。来源：[`OpenApi` docs](https://docs.rs/utoipa/6.0.0/utoipa/openapi/struct.OpenApi.html)。
- 需要从 axum path/query 自动推断参数时启用 `axum_extras` feature；类型映射 `uuid`/`chrono` 也各有 feature。来源：[utoipa README features](https://github.com/juhaku/utoipa/blob/master/README.md)。
- Swagger UI 可选：`utoipa-swagger-ui = { version = "10.0.2", features = ["axum"] }`，只在开发/内网启用，生产可关闭。来源：[utoipa-swagger-ui docs](https://docs.rs/utoipa-swagger-ui/10.0.2/utoipa_swagger_ui/)。

备选：`aide 0.15.1`。它同样只支持 OpenAPI 3.1.0，使用 `schemars 0.9` 生成 schema，提供 `aide::axum` 集成；如果团队更偏好“类型驱动 + 少宏”的风格可以选它。但当前 utoipa 的 axum router 集成更直接，release 也更活跃。来源：[aide docs](https://docs.rs/aide/0.15.1/aide/)、[crates.io aide](https://crates.io/crates/aide/0.15.1)。

### 6.2 TypeScript 客户端：orval 8 vs openapi-typescript

**推荐 orval 8.46.0**，原因：

- 前端 `shadcn-admin` 的 `package.json` 已使用 `@tanstack/react-query ^5.99.0`、`axios ^1.15.0`、React 19；orval 支持 `client: 'react-query'` 直接生成 TanStack Query hooks，并支持 custom axios mutator 注入 `Authorization` 和处理 401。来源：[shadcn-admin package.json](https://github.com/satnaing/shadcn-admin/blob/main/package.json)、[orval React Query 文档](https://github.com/orval-labs/orval/blob/master/docs/content/docs/guides/react-query.mdx)、[orval custom axios 文档](https://github.com/orval-labs/orval/blob/master/docs/content/docs/guides/custom-axios.mdx)。
- orval 内部会把 OpenAPI 2.0/3.0 升级到 3.1 处理，因此 utoipa 的 3.1 输出可用。来源：[orval input 文档](https://github.com/orval-labs/orval/blob/master/docs/content/docs/reference/configuration/input.mdx)。
- 注意 orval 8.46.0 的 engines 要求 **Node >= 22.18.0**；项目使用 bun，bun 通常兼容 Node API，但需要在目标 bun 版本上验证 `bunx orval`。如果 CI/本地 bun 不兼容，改用备选方案。来源：[npm orval 8.46.0](https://www.npmjs.com/package/orval/v/8.46.0)。

配置形状：

```ts
// orval.config.ts
import { defineConfig } from 'orval';

export default defineConfig({
  api: {
    input: './openapi.json',
    output: {
      target: './src/api/generated.ts',
      schemas: './src/api/model',
      client: 'react-query',
      httpClient: 'axios',
      override: {
        mutator: {
          path: './src/api/mutator.ts',
          name: 'customInstance',
        },
      },
    },
  },
});
```

`customInstance` 负责：

- `baseURL` 使用环境变量（不要硬编码生产地址）。
- 请求拦截器加 `Authorization: Bearer <access_token>`。
- 401 时最多刷新一次 token；刷新失败跳转登录页，避免无限重试。
- 不要把 refresh token 放在 localStorage；优先 httpOnly cookie（具体见安全专项调研）。

**备选：openapi-typescript 7.13.0 + openapi-fetch 0.17.0**：

- `openapi-typescript` 支持 OpenAPI 3.0/3.1，生成 runtime-free 的 `.d.ts`；`openapi-fetch` 是约 6 kB 的类型安全 fetch client。来源：[openapi-typescript 文档](https://openapi-ts.dev/introduction)、[openapi-fetch 文档](https://openapi-ts.dev/openapi-fetch/)。
- 适合不想引入 codegen 框架、只想拿类型的场景；但 TanStack Query hooks 需要自己包一层，鉴权/刷新逻辑也要自己写。
- 命令：
  ```bash
  bunx openapi-typescript ./openapi.json -o ./src/api/schema.d.ts
  ```
  来源：[openapi-typescript setup](https://openapi-ts.dev/introduction)。

### 6.3 生成流水线

建议把 OpenAPI 当成构建产物：

1. 后端集成测试/构建脚本生成 `openapi.json`（`ApiDoc::openapi().to_pretty_json()`），或由运行中的服务提供 `/api-docs/openapi.json`。
2. 前端仓库保存 `openapi.json`（或从后端 artifact 下载），运行 `bunx orval` / `bun run api:generate`。
3. 如果提交生成代码，CI 加 `git diff --exit-code` 检查漂移；如果不提交，CI 每次生成后再 `tsc --noEmit`。
4. 后端 CI 用 `utoipa` 的测试确保 OpenAPI 能序列化，并校验所有公开 handler 都有文档；前端用生成 client 做类型检查。
5. 鉴权端点（login/refresh/logout）单独标注 security；login 本身不需要 bearer，refresh 使用 cookie 或 refresh token。

注意：生成的 TypeScript 类型只提供编译期约束，不是运行时校验；后端仍必须用 `serde`/`validator` 做输入校验。OpenAPI 3.1 的 `nullable` 语义与 3.0 不同，utoipa 6 输出 3.1，orval/openapi-typescript 均支持 3.1，但仍建议在 CI 跑一次完整生成，避免 `Option<T>`/nullable 的边缘差异。

---

## 7. 无 Redis 的限流

### 7.1 tower-governor 0.8

`tower-governor` 基于 `governor` 的 GCRA（generic cell rate algorithm），提供按 IP、全局或自定义 key 的限流中间件。来源：[tower-governor README](https://github.com/benwis/tower-governor)、[governor docs](https://docs.rs/governor/0.10.4/governor/)。

依赖：

```toml
tower-governor = { version = "0.8.0", default-features = false, features = ["axum", "tracing"] }
```

**必须显式关闭默认 feature。** `tower-governor` 0.8.0 的默认 feature 是 `["axum", "tonic"]`；如果不需要 tonic，它会无谓地引入 `tonic 0.14` 及其依赖。来源：[crates.io tower-governor 0.8.0 features](https://crates.io/crates/tower-governor/0.8.0)。

配置语义（容易踩坑）：

```rust
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};

// 注意：per_second(60) 不是“每秒 60 个”，而是“每 60 秒补充 1 个 token”
let login_limit = GovernorConfigBuilder::default()
    .per_second(60)
    .burst_size(5)
    .finish()
    .unwrap();

let app = axum::Router::new()
    .route("/api/v1/auth/login", axum::routing::post(login))
    .route_layer(GovernorLayer::new(login_limit));
```

- `per_second(n)` 表示“每 n 秒补充一个元素”；`burst_size(m)` 是桶容量。上面的组合是“最多突发 5 次，之后平均每 60 秒一次”。来源：[tower-governor README](https://github.com/benwis/tower-governor#how-does-it-work)。
- `GovernorConfig::default()`：burst 8、每 500 ms 补充 1 个；`GovernorConfig::secure()`：burst 2、每 4 s 补充 1 个。来源：[tower-governor README](https://github.com/benwis/tower-governor#presets)。
- 默认发送 `retry-after` 和 `x-ratelimit-after`；调用 `.use_headers()` 后额外发送 `x-ratelimit-limit`、`x-ratelimit-remaining`、`x-ratelimit-whitelisted`。来源：[tower-governor README](https://github.com/benwis/tower-governor#add-x-ratelimit-headers)。
- 错误默认转成 429 响应；可以用 `GovernorLayer::error_handler` 自定义。来源：[tower-governor README](https://github.com/benwis/tower-governor#error-handling)。
- 限流状态按 key 常驻内存；长期运行应定期调用 `config.limiter().retain_recent()`（README 示例每 60 s 一次），否则大量不同 IP 会累积。来源：[tower-governor README](https://github.com/benwis/tower-governor#how-does-it-work)。

### 7.2 key extractor 与部署边界

- `PeerIpKeyExtractor` 是默认值，要求服务用 `into_make_service_with_connect_info::<SocketAddr>()` 启动；否则拿不到 peer IP，返回 `UnableToExtractKey`。来源：[tower-governor README](https://github.com/benwis/tower-governor#common-pitfalls)。
- `SmartIpKeyExtractor` 按 `x-forwarded-for`、`x-real-ip`、`forwarded` 的顺序取 IP，最后回退到 peer IP；**只有当应用位于可信反向代理之后，且代理会覆盖这些 header 时才安全**。否则客户端可以伪造 `X-Forwarded-For` 绕过限流。来源：[tower-governor README](https://github.com/benwis/tower-governor#customize-rate-limiting-key)。
- `GlobalKeyExtractor` 把所有请求计为一个 key，适合全局限流，不适合按用户。
- 自定义 key：实现 `KeyExtractor`（例如从 API key header 提取）。如果 key 需要从 JSON body 里的 username 提取，中间件层拿不到 body；应在 handler 里解析后，直接用 `governor::RateLimiter::keyed(quota)` + `check_key(&username_hash)` 做第二层限流。来源：[`RateLimiter` docs](https://docs.rs/governor/0.10.4/governor/struct.RateLimiter.html)。

### 7.3 多实例问题与无 Redis 方案

进程内限流的状态只存在于当前进程：

- 1 个副本：`tower-governor` 的限额就是实际限额。
- N 个副本：每个副本独立计数，攻击者轮询副本时实际限额约为 `N × 单副本限额`；负载均衡的 IP hash/连接复用会放大或缩小这个误差。

无 Redis 时的可选方案：

1. **单副本 + 进程内限流（MVP 首选）。** 结构最简单；在部署上明确“auth 服务暂时单副本”，并监控 CPU/内存。如果未来扩副本，必须升级方案。
2. **把限流放到 ingress/WAF/API Gateway。** 例如 nginx `limit_req_zone $binary_remote_addr zone=login:10m rate=5r/m;` + `limit_req zone=login burst=5 nodelay;`；云厂商的 WAF/API Gateway 也提供按 IP/路径的限流。优点是不占应用内存，所有应用副本共享同一层网关配额；但单机 nginx 的共享内存 zone 只在该主机的 worker 间共享，多网关实例时仍需要每实例配额或云 WAF 的全局配额。缺点是要维护网关配置，且应用层仍需防绕过/纵深。来源：[nginx limit_req](https://nginx.org/en/docs/http/ngx_http_limit_req_module.html)。
3. **PostgreSQL 原子计数。** 用 `INSERT ... ON CONFLICT DO UPDATE ... RETURNING` 实现固定窗口/滑动窗口计数：
   ```sql
   INSERT INTO auth_rate_limits (key, window_start, count)
   VALUES ($1, date_trunc('minute', now()), 1)
   ON CONFLICT (key, window_start)
   DO UPDATE SET count = auth_rate_limits.count + 1
   RETURNING count;
   ```
   优点：跨副本一致、无需 Redis；缺点：每次尝试都写数据库、热点行锁竞争、需要定期清理历史窗口，数据库故障时策略要明确（fail-open 还是 fail-closed）。适合登录这种低 QPS 场景，不适合高频 API。来源：[PostgreSQL `INSERT ... ON CONFLICT`](https://www.postgresql.org/docs/18/sql-insert.html#SQL-ON-CONFLICT)。
4. **sticky session / IP hash 负载均衡。** 只能减少“轮询副本”造成的误差，不能提供真正的全局限额；不推荐作为安全边界。

补充：`tower::limit::RateLimitLayer::new(num, per)` 是对**整个 service** 的全局速率限制，不区分调用方；`ConcurrencyLimitLayer` 限制 in-flight 请求数。它们不能替代 per-IP 限流，但可以用于保护下游资源（例如全局限 100 rps、并发限 50）。来源：[`RateLimitLayer` docs](https://docs.rs/tower/0.5.3/tower/limit/rate/struct.RateLimitLayer.html)、[`ConcurrencyLimitLayer` docs](https://docs.rs/tower/0.5.3/tower/limit/concurrency/struct.ConcurrencyLimitLayer.html)。

### 7.4 分层限流建议

登录端点建议至少三层：

1. **网关/边缘层**：按 IP 的粗粒度限制（例如 60 req/min/IP），挡住明显洪水。
2. **应用层 per-IP**：`tower-governor` 对 `/auth/login` 使用严格配额（例如 burst 5、每 60 s 补充 1 个）；对 `/auth/refresh` 稍宽松。
3. **per-account**：登录失败按 username（哈希后）计数，例如 5 次失败/15 分钟，之后指数退避或要求 CAPTCHA；成功登录后清零。不要因为“账号不存在”而返回不同错误，避免 user enumeration。
4. **资源层**：Argon2 并发 `Semaphore`；数据库连接池 `acquire_timeout`；全局 `ConcurrencyLimitLayer`。

`tower-governor` 0.8.0 最后一次发布是 2025-08-14，维护节奏一般；它是当前无 Redis 场景下最省事的 crate，但建议 pin 版本、关注 upstream，并保留替换为自研 `governor` 中间件或网关限流的余地。来源：[crates.io tower-governor](https://crates.io/crates/tower-governor/0.8.0)。

---

## 8. 可观测性：日志、metrics、health/readiness

### 8.1 结构化日志

依赖：

```toml
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter", "json"] }
```

初始化形状：

```rust
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

tracing_subscriber::registry()
    .with(EnvFilter::from_default_env()) // RUST_LOG
    .with(fmt::layer().json())
    .init();
```

- `json` layer 输出 newline-delimited JSON，适合容器 stdout 采集；`Json::flatten_event`、`with_current_span`、`with_span_list` 可调整结构。来源：[`Json` docs](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/fmt/format/struct.Json.html)。
- `EnvFilter::from_default_env()` 读取 `RUST_LOG`；`try_from_default_env()` 对非法 directive 返回错误。生产建议用 `try_from_default_env` 并在启动时失败，避免静默忽略配置。来源：[`EnvFilter` docs](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/filter/struct.EnvFilter.html)。
- 日志输出到 stdout，由容器 runtime/采集器收集；不要在应用内自己轮转文件（云原生环境通常由平台负责）。

HTTP 请求日志：

```toml
tower-http = { version = "0.7.1", features = ["trace", "request-id", "timeout", "cors"] }
```

```rust
use std::time::Duration;
use tower_http::{
    ServiceBuilderExt,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
};
use tracing::Level;

let middleware = tower::ServiceBuilder::new()
    // 先设置 request id，再进入 TraceLayer
    .set_x_request_id(MakeRequestUuid)
    .layer(
        TraceLayer::new_for_http()
            .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
            .on_response(DefaultOnResponse::new().level(Level::INFO)),
    )
    // 响应离开 TraceLayer 前把 request id 写回响应头
    .propagate_x_request_id();
```

- `TraceLayer::new_for_http()` 的 span 默认是 `Level::DEBUG`，`DefaultOnResponse` 默认也是 `DEBUG`；如果生产 `RUST_LOG=info`，不显式改成 INFO 就看不到请求日志。来源：[`DefaultMakeSpan` docs](https://docs.rs/tower-http/0.7.1/tower_http/trace/struct.DefaultMakeSpan.html)、[`DefaultOnResponse` docs](https://docs.rs/tower-http/0.7.1/tower_http/trace/struct.DefaultOnResponse.html)。
- `DefaultMakeSpan` 默认**不包含 headers**；不要为了调试打开 `include_headers(true)`，否则可能把 `Authorization`、`Cookie`、`Set-Cookie` 写进日志。来源：[`DefaultMakeSpan` docs](https://docs.rs/tower-http/0.7.1/tower_http/trace/struct.DefaultMakeSpan.html)。
- request id 的 layer 顺序必须正确：`SetRequestIdLayer` 在 `TraceLayer` 之前，`PropagateRequestIdLayer` 在响应路径上位于 `TraceLayer` 之后；tower-http 文档给出了完整顺序。来源：[tower-http request-id 文档](https://docs.rs/tower-http/0.7.1/tower_http/request_id/index.html)。
- 敏感字段要显式跳过：`#[tracing::instrument(skip(password, token, authorization))]`；不要记录完整请求体。来源：[`instrument` docs](https://docs.rs/tracing/0.1.44/tracing/attr.instrument.html)。

### 8.2 Metrics

依赖：

```toml
metrics = "0.24.6"
# 只要 recorder + 自己挂 /metrics 时，关闭默认的 http-listener/push-gateway
metrics-exporter-prometheus = { version = "0.18.3", default-features = false }
```

初始化：

```rust
use metrics_exporter_prometheus::PrometheusBuilder;

let handle = PrometheusBuilder::new().install_recorder()?;
// 必须周期性调用 run_upkeep，否则 histogram 数据会增长
let upkeep = handle.clone();
tokio::spawn(async move {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tick.tick().await;
        upkeep.run_upkeep();
    }
});
```

- `install_recorder()` 安装全局 recorder，返回 `PrometheusHandle`；**它不会自动 spawn upkeep**，调用方要周期性 `run_upkeep()`。来源：[`PrometheusBuilder::install_recorder` docs](https://docs.rs/metrics-exporter-prometheus/0.18.3/metrics_exporter_prometheus/struct.PrometheusBuilder.html#method.install_recorder)。
- `handle.render()` 生成 Prometheus exposition 文本，可在 `/metrics` handler 返回；`render_to_write` 可流式输出。来源：[`PrometheusHandle` docs](https://docs.rs/metrics-exporter-prometheus/0.18.3/metrics_exporter_prometheus/struct.PrometheusHandle.html)。
- 如果希望 exporter 自己起 HTTP listener，用 `PrometheusBuilder::new().with_http_listener(addr).install()?`，需要 `http-listener` feature；这会在独立端口响应任意 GET 路径。来源：[`with_http_listener` docs](https://docs.rs/metrics-exporter-prometheus/0.18.3/metrics_exporter_prometheus/struct.PrometheusBuilder.html#method.with_http_listener)。
- `metrics-exporter-prometheus` 默认 feature 是 `http-listener` + `push-gateway`，会拉入 hyper/rustls 等依赖；只用 recorder 时关闭默认 feature 可以减少依赖面。来源：[crates.io features](https://crates.io/crates/metrics-exporter-prometheus/0.18.3)。

建议指标：

- `http_requests_total{method, route, status}`：用 axum 的 `MatchedPath` 作为 `route`，避免把 UUID/邮箱写进 label 导致高基数。来源：[`MatchedPath` docs](https://docs.rs/axum/0.8.9/axum/extract/struct.MatchedPath.html)。
- `http_request_duration_seconds{method, route}` histogram。
- `db_pool_connections{state}` gauge（active/idle）。
- `auth_login_attempts_total{result, reason}`：`reason` 用有限枚举（bad_credentials、locked、rate_limited），不要带 username/email。
- `auth_password_hash_seconds` histogram（观察 Argon2 参数是否合适）。
- `process_*`、`tokio_*`（如 runtime metrics）可选。

`metrics` 宏（`counter!`、`gauge!`、`histogram!`、`describe_*!`）在未安装 recorder 时是 no-op；应在 `main` 最早期安装 recorder。来源：[metrics docs](https://docs.rs/metrics/0.24.6/metrics/)。

### 8.3 Health / readiness

建议两个端点：

- `GET /health/live`：进程存活即 200；**不要依赖数据库**。如果 liveness 因数据库故障失败，Kubernetes 会不断重启本来健康的进程，放大故障。
- `GET /health/ready`：检查数据库 `SELECT 1`（带 1–2 s timeout），失败返回 503；可再加“迁移已应用”检查，但不要执行 migration。Kubernetes readiness 失败会把 Pod 从 Service endpoint 移除，停止接流量。来源：[Kubernetes liveness/readiness/startup probes](https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/)。

实现要点：

```rust
let db_ok = matches!(
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&pool),
    )
    .await,
    Ok(Ok(1))
);
```

- 错误响应只返回 `{"status":"unready"}`，不要把 SQL 错误/连接串返回给调用方；详细错误写日志。
- `/metrics` 和 `/health/*` 建议放在内部端口或仅集群内可达；不要把 `/metrics` 暴露到公网。
- readiness 不要检查外部第三方（除非该依赖是硬依赖），避免级联失败。

### 8.4 优雅关闭

```rust
let listener = tokio::net::TcpListener::bind(addr).await?;
axum::serve(listener, app)
    .with_graceful_shutdown(shutdown_signal())
    .await?;
pool.close().await;
```

- `with_graceful_shutdown` 在 signal future 完成后停止接受新连接并等待 in-flight 请求；signal 通常监听 SIGTERM 和 Ctrl-C。来源：[axum `Serve::with_graceful_shutdown` docs](https://docs.rs/axum/0.8.9/axum/struct.Serve.html#method.with_graceful_shutdown)。
- server 返回后调用 `pool.close().await`，让 checked-out 连接归还并关闭；否则进程退出时可能留下未完成的查询。来源：[`Pool::close` docs](https://docs.rs/sqlx-core/0.9.0/sqlx_core/pool/struct.Pool.html#method.close)。
- 给关闭流程设置总 timeout（例如 10 s），避免长请求无限期拖住部署。

### 8.5 OpenTelemetry（可选）

如果后续需要 tracing backend（Jaeger/Tempo/OTLP），可以加：

```toml
tracing-opentelemetry = "0.34.0"
opentelemetry = "0.33.1"
opentelemetry-otlp = "0.33.1"
```

`tracing-opentelemetry 0.34.0` 依赖 `opentelemetry ^0.33.0`，与 `opentelemetry-otlp 0.33.1` 对齐。来源：[tracing-opentelemetry docs](https://docs.rs/tracing-opentelemetry/0.34.0/tracing_opentelemetry/)、[opentelemetry-otlp docs](https://docs.rs/opentelemetry-otlp/0.33.1/opentelemetry_otlp/)。

但 MVP 阶段 logs + Prometheus 已足够；OTel 会增加 exporter、batch、resource、采样等配置，建议等有明确 tracing 后端需求时再加。

---

## 9. PostgreSQL 18 集成测试

### 9.1 testcontainers 版本冲突（重要）

当前：

- `testcontainers` 最新稳定是 **0.28.0**（2026-08-06）。
- `testcontainers-modules` 最新是 **0.15.0**，其 `Cargo.toml` 依赖 `testcontainers ^0.27.0`，即 `>=0.27, <0.28`。

因此 **`testcontainers = "0.28"` + `testcontainers-modules = "0.15"` 不能直接共存**。两个选择：

1. **推荐**：`testcontainers = "0.27.3"` + `testcontainers-modules = { version = "0.15.0", features = ["postgres"] }`。
2. 用 `testcontainers 0.28.0` + `GenericImage` 手动定义 PostgreSQL 镜像，不依赖 modules crate；代码更多，但版本最新。

来源：[testcontainers-modules Cargo.toml](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/Cargo.toml)、[crates.io testcontainers](https://crates.io/crates/testcontainers)。

### 9.2 使用 modules 的 Postgres

`testcontainers-modules` 的 Postgres 默认镜像 tag 是 **`11-alpine`**（PG11 已 EOL），必须显式覆盖为 18：

```rust
use sqlx::postgres::PgPoolOptions;
use testcontainers::{ImageExt, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn login_rejects_bad_password() -> anyhow::Result<()> {
    let container = Postgres::default()
        .with_tag("18-alpine")      // 覆盖默认的 11-alpine
        .with_db_name("app_test")
        .with_user("app")
        .with_password("app")
        .start()
        .await?;

    let port = container.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://app:app@127.0.0.1:{port}/app_test");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    // ... 调用 handler / 直接查库断言 ...

    Ok(())
}
```

- `with_db_name`、`with_user`、`with_password`、`with_init_sql`、`with_fsync_enabled` 都是 modules 提供的方法；默认 db/user/password 都是 `postgres`。来源：[postgres module source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/postgres/mod.rs)。
- `with_tag("18-alpine")` 来自 `ImageExt`；模块测试里也有 `.with_tag("13-alpine")` 的用法。来源：[postgres module source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/postgres/mod.rs)。
- `get_host_port_ipv4(5432).await` 来自 `AsyncRunner`/`ContainerAsync`。来源：[testcontainers docs](https://docs.rs/testcontainers/0.27.3/testcontainers/)。
- 模块默认 `cmd` 会传 `-c fsync=off`，测试更快；需要真实 fsync 行为时用 `with_fsync_enabled()`。来源：[postgres module source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/postgres/mod.rs)。

**等待策略的坑**：模块的 ready condition 是等待日志行 `database system is ready to accept connections`。官方 `postgres` entrypoint 在初始化阶段会先启动一个只监听 Unix socket 的临时 server，再停止并启动最终 server；因此 ready condition 可能在临时 server 阶段就被满足，此时最终 server 尚未开始监听 TCP，测试客户端会短暂遇到 `connection refused`。实践上应在首次连接外包一层短重试/backoff（例如 10 次 × 200 ms），或使用自定义 wait strategy。来源：[postgres module source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/postgres/mod.rs)、[docker-library/postgres entrypoint](https://github.com/docker-library/postgres/blob/master/docker-entrypoint.sh)。

### 9.3 `#[sqlx::test]` 与 testcontainers 的组合

`#[sqlx::test]` 的隔离模型是“每个测试创建一个新数据库”，它需要 `DATABASE_URL` 指向一个有 CREATEDB 权限的用户；它会自动应用 `migrations/`。来源：[`#[sqlx::test]` docs](https://docs.rs/sqlx/0.9.0/sqlx/attr.test.html)。

矛盾点：`#[sqlx::test]` 在测试函数体执行**之前**就要建立管理连接，因此不能简单地在测试函数体里启动 testcontainers 再设置 `DATABASE_URL`。两种可行做法：

1. **CI/本地用外部 PostgreSQL**（推荐，最简单）：CI 用 GitHub Actions service container `postgres:18-alpine`，本地用 docker compose；设置 `DATABASE_URL` 后直接 `#[sqlx::test]`。每个测试自动建库，隔离好，代码最少。GitHub Actions 的 service container 用法见 [GitHub 文档](https://docs.github.com/en/actions/using-containerized-services/about-service-containers)。
2. **testcontainers + 手动建池**：不用 `#[sqlx::test]`，在测试里启动容器、建 pool、跑 `sqlx::migrate!`，每个测试用唯一 schema/数据库或事务回滚。适合需要完全自包含的本地测试，但需要自己处理隔离和 cleanup。

如果必须把 testcontainers 和 `#[sqlx::test]` 结合，可以在测试二进制启动时（例如 `#[ctor]` 或自定义 test harness）启动容器并设置 `DATABASE_URL`，但这会引入全局状态和并行测试竞态，不如直接用 CI service container。

### 9.4 CI 建议

Job A：offline metadata 校验

```yaml
services:
  postgres:
    image: postgres:18-alpine
    env:
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: postgres
      POSTGRES_DB: app
    ports: ["5432:5432"]
    options: >-
      --health-cmd "pg_isready -U postgres"
      --health-interval 5s
      --health-timeout 5s
      --health-retries 10
```

步骤：

1. `cargo install sqlx-cli --version 0.9.0 --no-default-features --features rustls,postgres`
2. `sqlx migrate run`（使用 `DATABASE_URL`）
3. `cargo sqlx prepare --check --workspace -- --all-targets --all-features`

Job B：构建 + 测试

1. `SQLX_OFFLINE=true cargo build --workspace --all-targets`
2. `SQLX_OFFLINE=true DATABASE_URL=... cargo test --workspace --all-targets`
3. 如果测试用 testcontainers，确保 runner 有 Docker（GitHub-hosted Linux runner 默认有）。

来源：[sqlx-cli README](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-cli/README.md)、[GitHub Actions service containers](https://docs.github.com/en/actions/using-containerized-services/about-service-containers)。

---

## 10. PostgreSQL 18 兼容性与 `uuidv7()`

### 10.1 协议与认证

- PostgreSQL 18 引入 wire protocol **3.2**（主要用于 256-bit cancel key 等），但服务器和 libpq 仍向后兼容 protocol **3.0**（PostgreSQL 7.4+）。来源：[PostgreSQL 18 protocol docs](https://www.postgresql.org/docs/18/protocol.html)。
- SQLx 0.9 的 PostgreSQL driver 仍发送 protocol `196608`（3.0）：`sqlx-postgres/src/message/startup.rs` 中 `buf.extend(&196_608_i32.to_be_bytes())`。因此 SQLx 连接 PG18 走的是 3.0 兼容路径，不需要 PG18 新协议特性。来源：[SQLx startup.rs](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-postgres/src/message/startup.rs)。
- 认证：SQLx 支持 SCRAM-SHA-256 / SCRAM-SHA-256-PLUS；PG18 的 `scram-sha-256` 是推荐方式。PG18 弃用 MD5 密码（未来会移除），新系统不要用 `md5`。来源：[PostgreSQL 18 auth-password docs](https://www.postgresql.org/docs/18/auth-password.html)、[PG18 release notes](https://www.postgresql.org/docs/18/release-18.html)。
- **PG18 新增的 `oauth` HBA method 不被 `sqlx-postgres` 支持**：SQLx 的 SASL 实现只匹配 SCRAM，遇到 `oauth` HBA 无法连接，当前 open issue #4399，且没有 out-of-tree workaround。使用托管 PG 时，如果提供商只提供 `oauth` 认证，需要换 driver/代理或等 SQLx 支持。来源：[transact-rs/sqlx#4399](https://github.com/transact-rs/sqlx/issues/4399)。
- TLS：生产建议 `sslmode=verify-full`，不要关闭证书校验；SQLx 的 `PgConnectOptions` 支持 TLS 配置。来源：[`PgConnectOptions` docs](https://docs.rs/sqlx/0.9.0/sqlx/postgres/struct.PgConnectOptions.html)。

### 10.2 `uuidv7()` 用法

PG18 新增 `uuidv7([shift interval]) → uuid`，生成时间有序 UUID；时间戳为 Unix ms + sub-ms + random，支持范围从 1970 到约 year 10889。还新增 `uuidv4()` 作为显式 v4 别名。来源：[PG18 UUID functions](https://www.postgresql.org/docs/18/functions-uuid.html)、[PG18 release notes](https://www.postgresql.org/docs/18/release-18.html)。

表结构示例：

```sql
CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    email text NOT NULL UNIQUE,
    password_hash text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
```

插入并取回：

```rust
let user = sqlx::query_as!(
    User,
    r#"
    INSERT INTO users (email, password_hash)
    VALUES ($1, $2)
    RETURNING id, email, created_at
    "#,
    email,
    password_hash,
)
.fetch_one(&pool)
.await?;
```

- SQLx 的 `uuid` feature 将 `uuid::Uuid` 映射到 PostgreSQL `UUID`；`chrono`/`time` feature 映射时间类型。来源：[SQLx Postgres types](https://docs.rs/sqlx/0.9.0/sqlx/postgres/types/index.html)、[crates.io sqlx features](https://crates.io/crates/sqlx/0.9.0)。
- UUIDv7 的 48-bit 毫秒时间戳 + 额外随机/子毫秒位提供索引局部性；RFC 9562 允许用 sub-ms/counter 提升同毫秒内 monotonicity，但不保证跨进程严格单调。不要把它当作业务排序键；需要严格顺序时用 `created_at` + 序列或业务字段。来源：[RFC 9562 §5.7](https://www.rfc-editor.org/rfc/rfc9562.html#section-5.7)。
- 如果需要在插入前就知道 ID（例如写事件/日志），或在 Rust 侧生成以便 schema 兼容 PG17，可用 `uuid` crate：
  ```toml
  uuid = { version = "1.29.0", features = ["v4", "v7", "serde"] }
  ```
  ```rust
  let id = uuid::Uuid::now_v7();
  ```
  `Uuid::now_v7()` 需要 `std` + `v7` feature；同一进程内生成的 UUID 按创建顺序有序。来源：[`Uuid::now_v7` docs](https://docs.rs/uuid/1.29.0/uuid/struct.Uuid.html#method.now_v7)。
- 选择策略：
  - **只面向 PG18+，追求简单**：DB `DEFAULT uuidv7()` + `RETURNING id`。
  - **需要回退到 PG17 或客户端提前生成**：Rust `Uuid::now_v7()`，数据库列只写 `uuid PRIMARY KEY`，不依赖 PG18 函数。
  - 不要把 `uuidv7()` 和 `created_at` 混用为两个“时间源”；`created_at` 用 `now()`，ID 只做标识/索引。

### 10.3 升级路径

- PG18 → 未来 PG19：`uuidv7()`、SCRAM、protocol 3.0 兼容路径都可持续；升级时仍建议先在 staging 用 `pg_upgrade`/逻辑复制验证。
- PG18 → 回退 PG17：如果 schema 使用 `DEFAULT uuidv7()`，回退会失败；需要先 `ALTER TABLE ... ALTER COLUMN id DROP DEFAULT` 并改由应用生成 UUID。这个决策应在第一版 schema 就确定。
- 测试矩阵：CI 至少跑 PG18；如果承诺 PG17 兼容，再加 PG17 job，并避免 `uuidv7()` 默认值。
- 托管 PostgreSQL：确认认证方式不是仅 `oauth`；如果是，SQLx 当前不可用（见 #4399）。

---

## 11. 风险与待确认项

1. **SQLx 0.9 较新**（2026-05 发布）且仓库迁移；如果遇到生态兼容问题，降级到 0.8.x 是可行退路，但需要重新验证 offline/migrate 行为。
2. **argon2 0.6.0 较新**（2026-08 发布），API 与 0.5.x 不同；如果依赖树里其他 crate 仍用 `password-hash 0.5`，可能需要暂时 pin `argon2 0.5.3`。新项目建议直接用 0.6。
3. **tower-governor 0.8.0 维护节奏一般**（最后发布 2025-08-14）；它是无 Redis 场景的务实选择，但多副本限流必须外部补足。
4. **testcontainers-modules 0.15 只兼容 core 0.27.x**；core 0.28 已发布但 modules 尚未跟进。若必须用 0.28，改用 `GenericImage`。
5. **orval 8.46.0 要求 Node >=22.18.0**；bun 的 Node 兼容性需要在项目实际 bun 版本上验证。若不兼容，改用 `openapi-typescript` + `openapi-fetch`。
6. **jsonwebtoken 标注 passively-maintained**，且不提供 HTTP JWKS 缓存；自签自验场景足够，接外部 IdP 时需要自己实现缓存/刷新。
7. **Metrics exporter 默认 feature 会拉入 HTTP listener/push gateway**；只用 recorder 时记得 `default-features = false`。
8. **`uuidv7()` 是 PG18-only**；如果“预留升级”包含回退能力，选择 Rust 侧生成。
9. 本文所有版本是 2026-11-05 的快照；落地前重新核验 crates.io/npm 与 docs.rs。

---

## 12. 主要来源

- Axum: [crates.io 0.8.9](https://crates.io/crates/axum/0.8.9), [serve docs](https://docs.rs/axum/0.8.9/axum/fn.serve.html), [0.8 changelog](https://github.com/tokio-rs/axum/blob/main/axum/CHANGELOG.md)
- SQLx: [0.9.0 changelog](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md), [sqlx-cli README](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-cli/README.md), [`query!` docs](https://docs.rs/sqlx/0.9.0/sqlx/macro.query.html), [`#[sqlx::test]` docs](https://docs.rs/sqlx/0.9.0/sqlx/attr.test.html)
- JWT: [jsonwebtoken README](https://github.com/Keats/jsonwebtoken/blob/v11.1.0/README.md), [josekit README](https://github.com/jedisct1/rust-josekit/blob/master/README.md), [RFC 7517](https://www.rfc-editor.org/rfc/rfc7517.html), [RFC 7518 §3.2](https://www.rfc-editor.org/rfc/rfc7518.html#section-3.2), [RFC 7519](https://www.rfc-editor.org/rfc/rfc7519.html), [RFC 8037](https://www.rfc-editor.org/rfc/rfc8037.html), [RFC 8725](https://www.rfc-editor.org/rfc/rfc8725.html)
- Argon2: [argon2 0.6.0 docs](https://docs.rs/argon2/0.6.0/argon2/), [argon2 changelog](https://github.com/RustCrypto/password-hashes/blob/argon2-v0.6.0/argon2/CHANGELOG.md), [RFC 9106 §4](https://www.rfc-editor.org/rfc/rfc9106.html#section-4)
- OpenAPI: [utoipa README](https://github.com/juhaku/utoipa/blob/master/README.md), [utoipa-axum docs](https://docs.rs/utoipa-axum/0.3.0/utoipa_axum/), [utoipa-swagger-ui docs](https://docs.rs/utoipa-swagger-ui/10.0.2/utoipa_swagger_ui/), [aide docs](https://docs.rs/aide/0.15.1/aide/)
- TypeScript: [orval React Query](https://github.com/orval-labs/orval/blob/master/docs/content/docs/guides/react-query.mdx), [orval custom axios](https://github.com/orval-labs/orval/blob/master/docs/content/docs/guides/custom-axios.mdx), [openapi-typescript](https://openapi-ts.dev/introduction), [openapi-fetch](https://openapi-ts.dev/openapi-fetch/)
- Rate limiting: [tower-governor README](https://github.com/benwis/tower-governor), [governor docs](https://docs.rs/governor/0.10.4/governor/), [nginx limit_req](https://nginx.org/en/docs/http/ngx_http_limit_req_module.html), [PostgreSQL ON CONFLICT](https://www.postgresql.org/docs/18/sql-insert.html#SQL-ON-CONFLICT)
- Observability: [tracing-subscriber Json](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/fmt/format/struct.Json.html), [EnvFilter](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/filter/struct.EnvFilter.html), [tower-http TraceLayer](https://docs.rs/tower-http/0.7.1/tower_http/trace/struct.TraceLayer.html), [metrics-exporter-prometheus](https://docs.rs/metrics-exporter-prometheus/0.18.3/metrics_exporter_prometheus/struct.PrometheusBuilder.html), [Kubernetes probes](https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/)
- Testcontainers: [testcontainers 0.27.3](https://crates.io/crates/testcontainers/0.27.3), [testcontainers-modules postgres source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/postgres/mod.rs), [GitHub Actions services](https://docs.github.com/en/actions/using-containerized-services/about-service-containers)
- PostgreSQL 18: [release notes](https://www.postgresql.org/docs/18/release-18.html), [protocol](https://www.postgresql.org/docs/18/protocol.html), [UUID functions](https://www.postgresql.org/docs/18/functions-uuid.html), [auth-password](https://www.postgresql.org/docs/18/auth-password.html), [RFC 9562 §5.7](https://www.rfc-editor.org/rfc/rfc9562.html#section-5.7)
