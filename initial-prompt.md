任何管理系统都始于登录，我需要你帮我制定一个实施方案，搭建生产可用登录系统，暂不需要写生产代码。
我指定一下技术栈：
- 前端参考这个repo：https://github.com/satnaing/shadcn-admin
	- fork后剪裁掉不需要的功能，保留dashboard、users、sigin in、settings
	- 增加简体中文语言支持
	- pnpm替换成bun
- 后端用 Rust 的 Axum 框架，用SQLx连数据库
- 数据库使用 PostgreSQL18，但预留升级。
- 授权方式采用JWT
