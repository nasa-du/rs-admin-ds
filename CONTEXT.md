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

**令牌派生 (Token Derivation)**:
刷新轮换时用上一枚原始令牌与轮换计数经 HMAC 派生下一枚原始令牌的方式；数据库仍只保存每枚令牌的 SHA-256。
_Avoid_: 令牌生成, refresh 算法

**会话淘汰 (Session Eviction)**:
用户活跃会话达到 5 个上限后，新登录自动撤销最久未活动会话的动作；写 `authn_session_evicted` 审计。
_Avoid_: 踢出, 强制下线

**重新认证 (Reauthentication)**:
已登录用户为满足敏感操作的最近认证要求而再次验证当前密码的动作；不建立新会话，只刷新会话的认证时间。
_Avoid_: 二次登录, step-up

**认证时间 (Auth Time)**:
用户最近一次通过密码成功完成认证的时间；敏感操作按它判断是否在允许的新鲜度窗口内。
_Avoid_: 登录时间

**会话引导 (Session Bootstrap)**:
应用冷启动时使用刷新令牌 Cookie 换取新的访问令牌并读取当前用户资料的过程；失败时视为未登录，不保留半恢复状态。
_Avoid_: 自动登录, 恢复登录

**单飞刷新 (Single-flight Refresh)**:
多个并发请求同时发现访问令牌失效时，共享同一次刷新请求，只触发一次令牌轮换。
_Avoid_: 并发刷新, 批量刷新

**重新认证对话框 (Reauthentication Prompt)**:
敏感操作返回 `reauth_required` 时统一弹出的当前密码验证对话框；成功后重试原操作一次，取消时不执行操作。
_Avoid_: 二次登录框, step-up 弹窗

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
管理员创建用户或重置凭据时生成或设置的短期密码；只保存哈希，明文只在下发响应中展示一次；用户下一次成功登录后必须先更改；到期未改密则失效，需要管理员重新重置。
_Avoid_: 初始密码, 默认密码, 一次性密码

**一次性临时密码展示 (One-time Temporary Password Display)**:
创建用户或重置凭据成功后只展示一次临时密码的面板；关闭后无法再次查看，密码不得进入缓存、日志或分析事件。
_Avoid_: 密码弹窗, 初始密码页

**常见密码列表 (Common Password List)**:
内置在应用中的离线弱密码集合，用于在设置或更改密码时拒绝常见与已泄漏的密码。
_Avoid_: 黑名单, 弱密码库

**必须改密 (Password Change Required)**:
用户持有临时密码时的状态；其会话只能读取自身信息、刷新令牌、改密和登出，其他业务 API 被拒绝。
_Avoid_: 首次登录, 强制登录

**受限会话 (Restricted Session)**:
由临时密码登录建立、受「必须改密」限制的会话。
_Avoid_: 临时会话, 未验证会话

**强制改密流程 (Forced Password Change Flow)**:
受限会话在完成改密前只能访问改密页面、刷新令牌、读取自身信息与登出的路由约束；改密成功后恢复完整访问。
_Avoid_: 首次登录页, 强制登录

**凭据重置 (Credential Reset)**:
管理员为其他用户签发新临时密码的动作；撤销目标用户的全部会话并递增其令牌版本，不适用于自己。
_Avoid_: 改密, 密码找回

**登录锁定 (Login Lockout)**:
因连续登录失败而临时拒绝登录的账号层状态；按用户计数，自动过期，不做永久自动锁定。
_Avoid_: 封号, 冻结, 停用

**解锁 (Unlock)**:
管理员清除某用户账号层登录锁定与失败计数的动作；不重置密码，不改变用户状态。
_Avoid_: 解封, 启用

**登录限流 (Login Throttling)**:
对登录失败按账号、IP 与全局三个作用域分别计数并拒绝后续尝试的机制；账号层表现为短期锁定，IP/全局层返回 429。
_Avoid_: 登录限制, 频率限制

**失败窗口 (Failure Window)**:
IP 与全局限流计数器所属的固定时间区间；窗口过期后计数从零开始。
_Avoid_: 冷却期, 时间窗

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

**权限显示名 (Permission Display Name)**:
权限目录中每个权限携带的中英文名称，前端按界面语言选择；缺失时回退原始权限 key。
_Avoid_: 权限标签, 本地化权限

**角色 (Role)**:
一组权限的命名集合，拥有稳定的 UUIDv7 标识；用户通过被分配的角色获得其中的权限。
_Avoid_: 用户组, 权限组

**内置角色 (Built-in Role)**:
`super_admin`、`admin`、`user` 三个由代码定义、权限集固定且不可编辑的角色。
_Avoid_: 系统角色, 默认角色

**内置角色名称 (Built-in Role Label)**:
内置角色在界面上的本地化名称，由前端文案键提供；稳定 code 同时以 chip 展示。
_Avoid_: 系统角色名, 角色别名

**自定义角色 (Custom Role)**:
由拥有角色管理权限的用户创建、权限集可编辑的角色；以 UUIDv7 标识、无 slug，权限只能取自权限目录。
_Avoid_: 业务角色

**基线角色 (Baseline Role)**:
每个用户自动拥有且不可移除的内置 `user` 角色；它不授予任何管理权限。
_Avoid_: 默认角色, 普通角色

**角色分配 (Role Assignment)**:
将用户拥有的角色整体替换为选定集合的管理动作；基线 `user` 始终保留且不可移除，`admin`/`super_admin` 仅超级管理员可授予或撤销，目标权限集必须满足授予子集规则。
_Avoid_: 改角色, 设置权限

**有效权限 (Effective Permissions)**:
用户全部角色权限的并集；超级管理员不受该集合限制。
_Avoid_: 最终权限, 合并权限

**权限门控 (Permission Gating)**:
前端根据当前用户有效权限决定导航、路由与操作是否渲染的规则；缺少动作权限时不渲染，目标级不可操作时禁用并说明原因；服务端始终权威。
_Avoid_: 前端鉴权, 权限隐藏

**超级管理员 (Super Admin)**:
内置 `super_admin` 角色；授权判定直接放行，自动拥有当前与未来新增的全部权限，但认证与会话有效性仍然适用。
_Avoid_: 超管, root

**授予子集规则 (Grant Subset Rule)**:
非超级管理员创建、修改或分配角色时，目标角色的权限集必须是其自身有效权限的子集；超级管理员豁免。
_Avoid_: 权限继承, 越权检查

**管理范围规则 (Management Scope Rule)**:
非超级管理员只能读取和管理有效权限不超出自身、且不持有内置 `admin` 或 `super_admin` 角色的用户；列表按该范围过滤，self 始终可见，管理动作不得作用于自己。
_Avoid_: 越权检查, 权限继承

**目标资格口径 (Target Eligibility)**:
`assignable` / `manageable` 只表达目标角色在授予或管理规则下是否合格，不包含操作者是否持有对应动作权限；前端再与 `/me` 权限求交集。
_Avoid_: 权限标志, 可分配

**角色软删除 (Role Soft Delete)**:
自定义角色在无任何用户分配（含停用用户）时的删除方式；记录保留用于审计与历史引用，但不再出现在角色列表或分配候选中，v1 不提供恢复。
_Avoid_: 停用角色, 归档角色

### 审计

**审计日志 (Audit Log)**:
系统对认证、用户、角色与系统事件追加写入的不可变记录；包含 actor、target、时间、IP、User-Agent、request_id 与结果，绝不记录密码、令牌或哈希。
_Avoid_: 操作日志, 系统日志

**审计级别 (Audit Severity)**:
审计事件的影响等级，取值为 info、warning、error 或 critical；决定告警与保留期。
_Avoid_: 日志级别, 严重性

**审计链 (Audit Chain)**:
审计记录按写入顺序通过带密钥的哈希值首尾相连形成的防篡改链；密钥位于应用密钥管理，不在数据库中。
_Avoid_: 日志链, 哈希链

**审计目标 (Audit Target)**:
审计事件所作用的对象；类型为 `user`、`role`、`session`、`audit_log` 或 `system`，携带可空 id 与名称快照。
_Avoid_: 操作对象, 资源

**目标名称快照 (Target Label)**:
事件发生时目标的可读名称快照；用户取 username，角色取 name；不随对象后续改名。
_Avoid_: 目标名称, 对象名

**审计访问 (Audit Access)**:
查看单条审计详情产生的 `audit_log_accessed` 审计事件；列表查询不产生。
_Avoid_: 日志查看, 审计浏览

**绝对时间窗 (Absolute Time Window)**:
审计查询把「最近 N 天」等预设解析为固定的 `from`/`to` UTC 区间并写入 URL，使翻页、刷新与分享使用同一窗口。
_Avoid_: 相对时间, 滚动窗口

### 客户端

**本地偏好 (Local Preference)**:
仅保存在浏览器、不同步到后端的用户设置，例如主题、字体、语言、通知与显示偏好；v1 的 Notifications/Display 属于此类。
_Avoid_: 用户偏好, 本地设置

**界面语言 (UI Locale)**:
用户在界面中看到的文案语言；v1 支持简体中文（`zh-CN`，默认）与英文（`en-US`），只作为本地偏好保存，不同步到后端。
_Avoid_: 语言设置, locale 配置

**错误码映射 (Error Code Mapping)**:
前端按后端稳定 `code` 选择当前界面语言文案的规则；后端返回的英文 `title`/`detail` 只作诊断，不作为主要文案。
_Avoid_: 错误翻译, 文案下发

### 部署

**同站子域 (Same-site Subdomains)**:
前端与 API 共享同一 registrable domain 但 origin 不同；本项目的默认部署假设（如 `admin.example.com` + `api.example.com`）。
_Avoid_: 同源, 跨站

**第一方 Cookie (First-party Cookie)**:
由当前站点（same-site）的 API 设置并随请求发送的 Cookie；同站子域部署下 refresh Cookie 属于此类。
_Avoid_: 第三方 Cookie

**可信代理 (Trusted Proxy)**:
被配置允许提供客户端 IP 信息的反向代理；只有来自可信代理的连接才采用其转发头中的客户端地址。
_Avoid_: 网关, 代理

**部署主机 (Deployment Host)**:
运行网关、前端静态站、API 与观测栈的单台 Linux 主机；PostgreSQL 可以运行在该主机，也可以由外部数据库承担。
_Avoid_: 服务器, 生产机

**网关 (Gateway)**:
终止 TLS、按主机名路由并施加边缘限流与安全响应头的反向代理；本系统使用 nginx 作为网关，同时是可信代理。
_Avoid_: 入口, 负载均衡

**外部数据库 (External Database)**:
通过 `DATABASE_URL` 指向、由部署主机之外的运维方管理的 PostgreSQL 实例；其备份、补丁与大版本升级由该运维方负责。
_Avoid_: 远程库, 托管库

**维护任务 (Maintenance Job)**:
由主机定时器触发的一次性容器任务，执行保留期清理、备份校验等操作；同一时刻只允许一个实例运行。
_Avoid_: 定时脚本, 清理任务

**部署状态 (Deployment State)**:
部署主机上记录当前已部署镜像 tag、镜像引用与 commit 的本地状态文件；下一次发布用它回滚上一 tag。
_Avoid_: 当前版本, 发布指针

**发布记录 (Release Record)**:
每次发布或发布演练写入状态目录的带时间戳 JSON 记录，包含操作者、镜像、迁移列表、就绪耗时、健康检查与回滚结果。
_Avoid_: 发布日志, 部署日志

**发布演练 (Release Dry Run)**:
执行迁移、部署与就绪检查后立即回滚到上一 tag 的发布验证；产生发布记录但不更新部署状态。
_Avoid_: 试发布, 预发布

**恢复演练 (Restore Drill)**:
从异地备份恢复数据库并验证迁移版本、关键数据与审计链的定期操作。
_Avoid_: 备份测试, 恢复测试

**升级演练 (Upgrade Drill)**:
在临时数据库实例上用备份数据预演 PostgreSQL 大版本升级与验证步骤的操作。
_Avoid_: 升级测试, 迁移演练
