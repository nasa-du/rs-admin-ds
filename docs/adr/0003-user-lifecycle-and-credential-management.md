# ADR-0003: 用户生命周期与凭据管理

- Status: accepted
- Date: 2026-11-05

## Context

「账号生命周期与登录流程」票据需要在无邮件服务、无 Redis、前后端分离的前提下，确定用户从创建到停用的完整生命周期、登录标识、临时密码与改密、锁定/解锁，以及首个超级管理员的初始化方式。ADR-0001 已固定令牌与会话模型，ADR-0002 已固定 RBAC 模型；本 ADR 在其上补充用户生命周期决策，并修订 ADR-0001 §5 的自助改密语义。

系统没有邮件服务，因此邀请链接、忘记密码与邮件通知都不可用；管理员必须通过线下渠道向用户交付临时密码。ASVS 6.4.6 建议管理员不应知道或选择用户密码；本 ADR 按产品要求提供「管理员直接设置密码」能力，并用临时密码、强制改密、短期过期、会话撤销与审计来限制风险。这是有意偏离，必须在安全评审中记录。

## Decision

1. **生命周期与状态**
   - 用户状态只有 `active` 与 `disabled`；v1 不提供硬删除、软删除或 `pending` 状态。
   - `must_change_password` 与 `temporary_password_expires_at` 是独立字段；「待首次登录」与「临时密码已过期」由这两个字段派生，不新增状态。
   - 停用立即撤销全部会话并递增 `token_version`；重新启用只恢复登录资格，不恢复旧会话。停用期间保留 username/email 唯一性、角色分配与审计引用。
   - 重新启用后，未过期的临时密码继续有效；已过期则必须由管理员重置凭据。
   - 禁止自我停用与自我降级；最后一位启用的超级管理员受 ADR-0002 保护。

2. **登录标识**
   - `username` 与 `email` 都是必填的登录标识，各自唯一（包含停用用户）。
   - `username`：3–32 字符，ASCII `[a-z0-9][a-z0-9._-]*`，不允许 `@`、空白或 Unicode；存储前转小写。保留 `admin`、`root`、`system` 等少量名称防止混淆。
   - `email`：`trim`、Unicode NFC、转小写后存储；格式与长度由 API 校验。
   - `display_name` 可选，缺省为 username；支持 Unicode，仅用于展示。`phone` 为可选资料字段，不参与登录、不要求唯一，仅管理员维护。
   - 登录输入先 `trim`：含 `@` 按 email 查找，否则按 username 查找。两者规范化后匹配；不允许 username 含 `@` 保证无歧义。
   - 账号层失败计数按解析后的 `user_id` 记录；未知标识按 `HMAC(规范化标识)` 记录，避免攻击者在 email/username 之间轮换以绕过锁定。
   - username/email 只能由管理员修改（`users.update`），修改立即改变登录标识并写审计；JWT 不包含二者，因此不撤销会话。用户自助只能修改显示名与密码。

3. **创建与临时密码**
   - 创建用户与管理员重置凭据统一签发临时密码；可以由系统生成，也可以由管理员手动输入。
   - 系统生成：24 字符 base64url 随机串（约 144-bit），只在创建/重置响应中返回一次。
   - 管理员手动输入：必须通过密码策略（当前基线为 15–128 字符、NFC、常见/泄漏密码 blocklist、不截断），并同样只存 Argon2id。
   - 无论生成或手输，均设置 `must_change_password = true` 与 `temporary_password_expires_at = now() + 72h`（默认值，可配置）；密码值不写日志、不写入审计。
   - 管理员重置凭据时，先撤销目标用户全部会话、递增 `token_version`，并清除账号层登录锁定；管理员不能对自己执行重置。
   - **v1 不提供管理员设置永久密码的能力**；所有管理员设置/重置的密码都是临时密码。管理员不能查看既有密码，也不能再次读取已签发的临时密码。
   - 临时密码在改密或到期前可重复用于登录；每次登录都会建立受限会话。

4. **首次登录强制改密**
   - 临时密码登录成功后，服务端创建正常会话与 access token，但该用户处于 `must_change_password` 状态。
   - 受限会话只允许：`GET /auth/me`、`POST /auth/refresh`、`POST /auth/change-password`、`POST /auth/logout`；其他受保护端点返回 403 `password_change_required`。
   - 改密要求当前密码（即临时密码）与新密码；成功后清除 `must_change_password` 与 `temporary_password_expires_at`，撤销其他全部会话、递增 `token_version`，并为当前会话重签 access token。当前设备不掉线（修订 ADR-0001 §5）。
   - 临时密码过期只阻止再次登录；已存在的受限会话仍可改密，直到会话按 idle/absolute 规则失效。过期后再次登录返回 403 `temporary_password_expired`（仅在密码验证通过后返回，避免枚举），需要管理员重新重置。
   - 管理员重置凭据、停用用户或全部登出仍撤销全部会话并递增 `token_version`。

5. **登录与锁定**
   - 登录先解析标识并检查账号层/IP 层锁定；锁定期间仍执行一次 dummy Argon2id 验证，返回 429 与 `Retry-After`，即使密码正确也不放行。
   - 未知标识与错误密码都执行 Argon2id/dummy 验证并返回统一的 401 文案与错误码；不得通过状态码、文案或响应时间暴露用户是否存在。
   - 密码正确但用户停用 → 403 `account_disabled`；密码正确但临时密码过期 → 403 `temporary_password_expired`；这两类情况不增加失败计数。
   - 成功登录重置账号层失败计数，保留 IP 层计数；创建会话并签发令牌。
   - 账号层采用指数退避：5 次失败锁 30 秒、10 次锁 5 分钟、15 次锁 1 小时；不做永久自动锁定。IP 层与全局限流阈值由「安全基线：密码、限流、审计事件」票据决定，状态存 PostgreSQL，不使用进程内存。
   - 新增权限 `users.unlock`；内置 `admin` 拥有该权限，`super_admin` 自动拥有，自定义角色可被授予。解锁只清除账号层失败计数与锁定，不动 IP/全局计数，不重置密码；受管理范围规则约束（普通管理员不能解锁持有 `admin`/`super_admin` 角色的用户，超级管理员可以），任何人不得解锁自己；审计 `user_unlocked`。
   - 管理员重置凭据也会清除账号层锁定；v1 不提供独立的「忘记密码」流程。

6. **首个超级管理员与应急恢复**
   - 首个超级管理员通过 CLI 初始化：`bootstrap-super-admin --email <email> --username <username>`；仅当系统没有启用的超级管理员时允许执行。
   - 邮箱已存在时提升该用户、启用并分配 `super_admin`；邮箱不存在时创建新用户。若用户名冲突则失败。
   - 密码通过交互式无回显提示或 `--password-stdin` 读取；**不接受 `--password` 参数**，避免 shell history 与进程列表泄漏；创建的凭据是临时密码，首次登录强制改密。
   - 启动过程与数据库迁移绝不自动创建或重置超级管理员；CLI 不暴露为 HTTP 端点。
   - `recover-super-admin --email <email>` 作为 break-glass 恢复：要求直接数据库访问与交互确认，目标必须已持有 `super_admin`；重新签发临时密码、启用账号、撤销全部会话并递增 `token_version`；写审计。用于临时密码过期或全部超级管理员凭据丢失。
   - 引导与恢复都必须保持 ADR-0002 的「至少一位启用的超级管理员」不变量。

7. **Users 页面 CRUD 边界**
   - 允许：创建用户（含临时密码）、编辑显示名/username/email/phone、启用/停用、重置凭据、解锁、分配角色、查看。
   - 不允许：硬删除、软删除、批量删除、邀请/邮件流程、管理员设置永久密码、查看既有密码或已签发临时密码。
   - 创建时自动分配基线 `user` 角色；其他角色通过独立的分配角色动作，或由创建请求原子携带；后者必须走授予子集规则与管理范围规则（具体 schema 由「API 与数据契约」决定）。
   - 自助端点只允许修改显示名与密码；username/email/phone 由管理员维护。
   - 所有变更动作受 ADR-0002 的管理范围规则、自我操作禁令与超级管理员不变量约束；具体 UI 与批量启用/停用由「Users 页面真实 CRUD 集成」决定。
   - 重置凭据、解锁、停用等敏感操作是否要求 step-up，由「安全基线：密码、限流、审计事件」票据决定。

8. **审计事件**
   - 用户生命周期事件至少包括：`user_created`、`user_updated`、`user_status_changed`、`user_password_reset`（记录 `method=generated|manual`）、`user_unlocked`、`user_roles_changed`、`authn_password_changed`。
   - 认证事件至少包括：`authn_login_success`、`authn_login_fail`、`authn_login_lock`、`authn_logout`、`authn_logout_all`。
   - 系统事件：`system_bootstrap_super_admin`、`system_recover_super_admin`。
   - 所有事件记录 actor、target、时间、IP、User-Agent、request_id 与结果；绝不记录密码、密码哈希、临时密码、access/refresh token。失败登录只记录解析后的 `user_id` 或 `HMAC(标识)`，不记录原始密码。
   - 事件字段 schema、严重级别、不可篡改存储与保留期由「安全基线：密码、限流、审计事件」与「API 与数据契约」票据决定。

## Consequences

- **正面**：不依赖邮件服务；管理员可以线下交付临时密码；双登录标识让用户可用 email 或 username；停用、重置与改密对会话的撤销即时生效；管理员可以解锁与 break-glass 恢复；无删除简化了唯一约束、角色分配与审计引用。
- **负面**：管理员会知道或选择临时密码，这是对 ASVS 6.4.6 的有意偏离；缓解是强制改密、72 小时过期、会话撤销、密码策略、管理范围与完整审计。临时密码必须线下交付，存在聊天记录/截屏泄漏风险。双登录标识扩大了凭据面，必须共享同一账号级锁定计数。72 小时临时密码窗口比一般邀请链接更长，需要在部署票据中确认可配置范围。
- **复查条件**：引入邮件服务 → 重新评估邀请/重置链接，移除管理员直接设密；安全评审不接受管理员知道密码 → 改为仅系统生成临时密码或一次性链接；需要合规删除 → 新增软删除 ADR 并重新评估唯一约束；step-up 决策改变 → 更新 ADR-0001 与 ADR-0002；需要更严格的锁定策略 → 由安全基线票据调整阈值。

## Considered Options

- **邀请令牌而非临时密码**：管理员不接触密码，但需要未认证的设置端点、pending 状态与链接交付；用户选择临时密码，v1 不选。
- **管理员设置永久密码**：操作简单，但管理员长期知道密码且没有强制轮换；不选。
- **只允许 email 登录**：界面简单，但用户已要求 username/email 任一登录；不选。
- **软删除用户**：保留「已删除」隐藏态，但与邮箱/用户名唯一约束、角色删除规则和审计引用纠缠；v1 不选。
- **迁移 seed 或启动时环境变量创建超管**：会把默认凭据带进迁移历史或每次启动；不选，改用 CLI。
- **永久自动锁定**：更严格，但可被用于拒绝服务攻击；不选，改用短期指数退避与管理员解锁。
- **复用 `users.set_status` 做解锁**：减少权限项，但解锁与状态变更语义不同，审计与范围控制不清晰；不选。

## References

- [账号生命周期与登录流程](https://github.com/nasa-du/rs-admin-ds/issues/7)
- [ADR-0001: 认证令牌与会话模型](./0001-auth-token-session-model.md)
- [ADR-0002: RBAC 权限模型与授权规则](./0002-rbac-permission-model.md)
- `docs/research/auth-security.md`
- `CONTEXT.md`
