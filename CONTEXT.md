# rs-admin-ds

后台管理系统的登录、会话与权限上下文；前端为 `satnaing/shadcn-admin` 的裁剪 fork，后端为 Rust Axum + SQLx + PostgreSQL 18。

## Language

### 认证与会话

**用户 (User)**:
后台系统中的一个操作者身份，拥有稳定的 UUIDv7 标识；登录标识与启用状态由用户生命周期决定。
_Avoid_: 账号, 账户, 操作员

**会话 (Session)**:
用户一次成功登录后在服务端建立的会话记录；`auth_sessions.id` 同时是访问令牌的 `sid` 和刷新令牌族的边界。
_Avoid_: 登录记录

**访问令牌 (Access Token)**:
短寿命 JWT，用于业务 API 的 Bearer 认证；只携带用户、会话和令牌版本，不携带角色或权限。
_Avoid_: token, 登录 token

**刷新令牌 (Refresh Token)**:
高熵不透明随机串，只通过 `__Host-refresh` HttpOnly Cookie 传输；服务端只保存其 SHA-256 哈希，并在每次刷新时轮换。
_Avoid_: refresh JWT, 长期 token

**令牌族 (Token Family)**:
同一会话内由刷新轮换产生的刷新令牌链；检测到旧令牌在 grace 窗口外重放时整族撤销。
_Avoid_: token 链, 会话链

**令牌版本 (Token Version)**:
`users.token_version` 整数；递增后该用户此前签发的全部访问令牌立即失效。
_Avoid_: version, 版本号

**撤销 (Revoke)**:
在服务端将会话或令牌标记为不可用；仅清除浏览器 Cookie 不构成撤销。
_Avoid_: 注销, 删除, 清除

**登出 (Logout)**:
撤销当前会话并清除 refresh Cookie 的用户动作；不撤销该用户的其他会话。
_Avoid_: 注销, 退出登录

**全部登出 (Logout All)**:
撤销该用户全部会话并递增令牌版本的动作；用户需要重新登录。
_Avoid_: 全局登出

**刷新重放 (Refresh Replay)**:
已轮换的刷新令牌在 grace 窗口外再次出现；被视为令牌泄漏，撤销整个令牌族。
_Avoid_: 重试, 重复刷新

**Grace 窗口 (Grace Window)**:
刷新轮换后允许旧令牌在 30 秒内幂等重试的时间窗；窗口内返回同一新令牌，窗口外按刷新重放处理。
_Avoid_: 宽限期

### 用户生命周期

**用户生命周期 (User Lifecycle)**:
用户从创建、临时密码登录与改密、日常登录、停用到重新启用的过程；v1 不包含删除。
_Avoid_: 账号生命周期

**用户状态 (User Status)**:
用户是否被允许登录的状态，取值为启用（`active`）或停用（`disabled`）；停用是可逆的管理动作，不是删除。
_Avoid_: 账号状态, 启用标志

**停用 (Disable)**:
将用户置为不可登录并撤销其全部会话的管理动作；重新启用只恢复登录资格，不恢复旧会话。
_Avoid_: 禁用, 冻结, 删除

**登录标识 (Login Identifier)**:
用于认证的标识，可以是 email 或 username；两者规范化后各自唯一，登录时任选其一。
_Avoid_: 账号名, 登录名

**用户名 (Username)**:
登录标识之一；ASCII、小写、无 `@`，与显示名分离。
_Avoid_: 昵称, 账号

**邮箱 (Email)**:
登录标识之一；规范化小写后唯一。
_Avoid_: 邮件地址

**显示名 (Display Name)**:
用于界面展示的名称，可包含中文，不参与登录，不要求唯一；缺省为用户名。
_Avoid_: 昵称, 姓名

**电话 (Phone)**:
可选的用户资料字段，不参与登录，不要求唯一，由管理员维护。
_Avoid_: 手机登录, 联系方式

**临时密码 (Temporary Password)**:
管理员创建用户或重置凭据时生成或设置的短期密码；只保存哈希，用户下一次成功登录后必须先更改；到期未改密则失效，需要管理员重新重置。
_Avoid_: 初始密码, 默认密码, 一次性密码

**必须改密 (Password Change Required)**:
用户持有临时密码时的状态；其会话只能读取自身信息、刷新令牌、改密和登出，其他业务 API 被拒绝。
_Avoid_: 首次登录, 强制登录

**受限会话 (Restricted Session)**:
由临时密码登录建立、受「必须改密」限制的会话。
_Avoid_: 临时会话, 未验证会话

**凭据重置 (Credential Reset)**:
管理员为其他用户签发新临时密码的动作；撤销目标用户的全部会话并递增其令牌版本，不适用于自己。
_Avoid_: 改密, 密码找回

**登录锁定 (Login Lockout)**:
因连续登录失败而临时拒绝登录的账号层状态；按用户计数，自动过期，不做永久自动锁定。
_Avoid_: 封号, 冻结, 停用

**解锁 (Unlock)**:
管理员清除某用户账号层登录锁定与失败计数的动作；不重置密码，不改变用户状态。
_Avoid_: 解封, 启用

**引导超级管理员 (Bootstrap Super Admin)**:
系统没有启用的超级管理员时，通过 CLI 创建或提升的第一个超级管理员。
_Avoid_: 种子管理员, 默认管理员

**应急恢复 (Break-glass Recovery)**:
凭据丢失或临时密码过期时，通过直接数据库访问的 CLI 为既有超级管理员重新签发临时密码的应急动作。
_Avoid_: 后门, 绕过登录

### 授权

**权限 (Permission)**:
一个命名能力，key 形如 `resource.action`；权限目录中的一项。
_Avoid_: 特权, scope

**权限目录 (Permission Catalog)**:
系统中全部权限的权威清单，由代码定义并携带中英文显示名与分组。
_Avoid_: 权限列表, 权限表

**角色 (Role)**:
一组权限的命名集合，拥有稳定的 UUIDv7 标识；用户通过被分配的角色获得其中的权限。
_Avoid_: 用户组, 权限组

**内置角色 (Built-in Role)**:
`super_admin`、`admin`、`user` 三个由代码定义、权限集固定且不可编辑的角色。
_Avoid_: 系统角色, 默认角色

**自定义角色 (Custom Role)**:
由拥有角色管理权限的用户创建、权限集可编辑的角色；以 UUIDv7 标识、无 slug，权限只能取自权限目录。
_Avoid_: 业务角色

**基线角色 (Baseline Role)**:
每个用户自动拥有且不可移除的内置 `user` 角色；它不授予任何管理权限。
_Avoid_: 默认角色, 普通角色

**有效权限 (Effective Permissions)**:
用户全部角色权限的并集；超级管理员不受该集合限制。
_Avoid_: 最终权限, 合并权限

**超级管理员 (Super Admin)**:
内置 `super_admin` 角色；授权判定直接放行，自动拥有当前与未来新增的全部权限，但认证与会话有效性仍然适用。
_Avoid_: 超管, root

**授予子集规则 (Grant Subset Rule)**:
非超级管理员创建、修改或分配角色时，目标角色的权限集必须是其自身有效权限的子集；超级管理员豁免。
_Avoid_: 权限继承, 越权检查

**管理范围规则 (Management Scope Rule)**:
非超级管理员只能读取和管理有效权限不超出自身、且不持有内置 `admin` 或 `super_admin` 角色的用户；列表按该范围过滤，self 始终可见，管理动作不得作用于自己。
_Avoid_: 越权检查, 权限继承

**角色软删除 (Role Soft Delete)**:
自定义角色在无任何用户分配（含停用用户）时的删除方式；记录保留用于审计与历史引用，但不再出现在角色列表或分配候选中，v1 不提供恢复。
_Avoid_: 停用角色, 归档角色

### 审计

**审计日志 (Audit Log)**:
系统对认证、用户、角色与系统事件追加写入的不可变记录；包含 actor、target、时间、IP、User-Agent、request_id 与结果，绝不记录密码、令牌或哈希。
_Avoid_: 操作日志, 系统日志

### 部署

**同站子域 (Same-site Subdomains)**:
前端与 API 共享同一 registrable domain 但 origin 不同；本项目的默认部署假设（如 `admin.example.com` + `api.example.com`）。
_Avoid_: 同源, 跨站

**第一方 Cookie (First-party Cookie)**:
由当前站点（same-site）的 API 设置并随请求发送的 Cookie；同站子域部署下 refresh Cookie 属于此类。
_Avoid_: 第三方 Cookie
