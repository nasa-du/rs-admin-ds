# ADR-0002: RBAC 权限模型与授权规则

- Status: accepted
- Date: 2026-11-05

## Context

管理后台需要内置三角色（超级管理员/管理员/普通用户）、自定义角色、权限勾选与用户多角色。ADR-0001 已决定访问令牌不承载角色与权限，每个受保护请求从 PostgreSQL 解析；不引入 Redis。本 ADR 记录 RBAC 的领域模型与授权边界，供 API、前端与安全基线票据实现。

## Decision

1. **权限目录**：权限由后端代码定义，key 为 `resource.action`（资源用复数、action 用 snake_case），带中英文显示名与分组；数据库 `permissions` 表只是镜像，只在迁移/启动时同步，不接受运行时创建。v1 目录为 `users.read`、`users.create`、`users.update`、`users.set_status`、`users.reset_password`、`users.unlock`、`users.assign_role`、`sessions.revoke`、`roles.read`、`roles.create`、`roles.update`、`roles.delete`、`audit.read`。自助操作（自己的显示名、改自己的密码、dashboard）只要求已认证，不进入目录。
2. **内置角色**：`super_admin`、`admin`、`user` 由代码定义，权限集固定，不可创建/编辑/删除。`user` 是每个用户自动拥有且不可移除的基线角色，不授予管理权限。`admin` 初始权限为 `users.read`、`users.create`、`users.update`、`users.set_status`、`users.reset_password`、`users.unlock`、`users.assign_role`、`sessions.revoke`、`roles.read`、`audit.read`。`super_admin` 在授权判定中直接放行，自动拥有当前与未来全部权限；认证与会话有效性仍然适用，step-up 由安全基线票据决定。
3. **多角色**：有效权限为所有角色权限的并集，v1 不支持 deny、优先级或冲突规则。角色变更在下一个请求生效；RBAC 变更不递增 `token_version`、不撤销会话。
4. **授予与范围**：非超级管理员受授予子集规则约束——创建、修改或分配角色时，目标权限集必须是自身有效权限的子集；超级管理员豁免。非超级管理员只能管理有效权限不超出自身且不持有内置 `admin`/`super_admin` 角色的用户。`admin` 与 `super_admin` 角色的授予/撤销仅超级管理员；管理端点不得作用于自己；禁止自我降级与自我停用。
5. **自定义角色**：角色 id 为 UUIDv7；自定义角色无 slug，显示名在活跃角色中不区分大小写唯一、可编辑，描述可选；权限至少一项且必须来自目录；不能设置超级管理员标志。
6. **删除**：自定义角色仅在无任何用户分配（含停用用户）时允许软删除；删除后从列表与分配候选中隐藏、不可再分配，v1 不恢复；记录保留用于审计与历史引用。内置角色不可删除。
7. **超级管理员不变量**：系统至少有一个启用的超级管理员；最后一位不能被降级、停用或禁用；禁止自我降级；首个超级管理员由「账号生命周期与登录流程」票据初始化。
8. **权限解析与前端**：v1 每请求从数据库解析，不缓存，fail-closed（与 ADR-0001 一致）；`/me` 返回有效权限与角色，前端复用同一目录门控菜单与按钮；后端始终权威，无权限显示 403。
9. **敏感操作**：角色/权限变更、超级管理员变更、用户停用、凭据重置、解锁、审计查询标记为敏感操作；是否要求 step-up 由「安全基线：密码、限流、审计事件」票据决定，本 ADR 不改变 ADR-0001 的令牌模型。

## Amendments

- 2026-11-05：权限目录新增 `users.unlock`，内置 `admin` 权限集包含该项（见 ADR-0003）；普通管理员只能解锁不持有 `admin`/`super_admin` 角色的其他用户，超级管理员可解锁任意用户，任何人不得解锁自己。自助操作措辞统一为「显示名与密码」；v1 不提供用户删除。

## Consequences

- **正面**：权限与角色不进入 JWT，RBAC 变更即时生效；代码拥有目录避免漏配；并集模型简单可预测；软删除保留审计历史。
- **负面**：每请求一次数据库授权查询；超级管理员绕过不出现在权限表，需要 UI 明示；不支持 deny/ABAC，复杂例外需新增模型；软删除角色的名称可复用，审计需依赖 id 快照。
- **复查条件**：出现 deny、字段级或资源级授权需求时重新评估 RBAC/ABAC；权限目录需要跨服务共享时引入独立权限服务或 JWKS 式分发；数据库压力需要缓存时引入 5–15 秒缓存并文档化撤销窗口；step-up 决定改变令牌模型时更新 ADR-0001。

## Considered Options

- **超级管理员显式绑定全部权限**：更可审计，但新增权限需回填，漏配会产生隐蔽的权限缺口；不选。
- **支持 deny/冲突优先级**：更灵活，但解析复杂、UI 难以解释；v1 不选。
- **内置角色可编辑**：灵活，但会破坏三档角色的稳定语义；不选。
- **硬删除角色**：简单，但破坏审计与历史引用；不选。
- **自定义角色使用用户输入 slug**：便于外部引用，但中文名转写成本高；不选。
- **v1 引入权限缓存**：降低数据库压力，但与 ADR-0001 的即时生效目标冲突；不选，留作复查条件。

## References

- [RBAC 权限模型与授权规则](https://github.com/nasa-du/rs-admin-ds/issues/6)
- [ADR-0001: 认证令牌与会话模型](./0001-auth-token-session-model.md)
- [ADR-0003: 用户生命周期与凭据管理](./0003-user-lifecycle-and-credential-management.md)
- `CONTEXT.md`
