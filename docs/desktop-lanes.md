# 桌面工程 lane board

本表记录目录写入所有权，避免 Rust/前端两个组件同时改锁或接口。正式分支 `feat/tauri-desktop-app` 位于独立 worktree `open-meeting-memo-desktop`，起点 `origin/main` 的 `51319b733cc7c6b01ff39a3a627af95c9da69620`。原主树与 `open-meeting-memo-webui-demo` 只读。只做本地组件交接提交，最终全量测试与正式评审前禁止 push；PR 不自动合并。

| 顺序 / lane | 当前状态 | 独占写入 | 交付条件 |
|---|---|---|---|
| 20 Plan | Main 已批准 | 计划与内部裁决 | 不再另起不兼容架构 |
| 21 Foundation | 已交接 durable Foundation | 本提交建立共享文件 | 契约、deny-default 宿主、锁、脚本、文档、目标验证，本地 durable commit |
| 22 Native | 组件 durable 已交接；缺陷回原 owner | `src/**`（生成 DTO 除外）、Rust `tests/**`、根 Cargo.toml/Cargo.lock、`apps/desktop/src-tauri/**`、`packaging/desktop/**`、`docs/desktop-native.md` | 全部真实共享用例/CLI/宿主/媒体/版本门禁/资源打包/独立fixture测试与 durable commits |
| 22 Frontend | 组件 durable 已交接；缺陷回原 owner | `apps/desktop/src/**`（生成 DTO 除外）、`public/**`、前端 `tests/**`、前端配置/package.json/package-lock.json、`docs/desktop-ui.md` | 全部设计功能接 typed bridge、TS/DOM/交互/共享桥测试与 durable commits |
| 23 Integration-only | 组件已去重合入；真实 WK 缺陷修复中 | 共享 scripts/CI、根 README/AGENTS/.gitignore、安装/设计/desktop文档 | 只整合已完成组件；组件缺陷退回原owner，不接手缺失主功能 |
| 24 Final acceptance | 尚未开始 | 冻结后修复回owner | 全量门禁、fresh正式复审、release资源/签名、真实用户级安装/启动、SSH push＋GitHub PR |

## 冻结共享文件

- `contracts/desktop/v1/**`、`scripts/generate-desktop-contracts.mjs`、`src/application/desktop_contracts.rs`、`apps/desktop/src/contracts.generated.ts` 不属于组件自由修改范围。
- schema/generated 的必要变更先内部 Main 指定单 owner，带两个组件兼容更新与全契约重跑；不得单侧手改生成文件。根 Cargo 锁完全归 Native，npm 锁完全归 Frontend。
- Foundation 的 `tests/desktop_contracts.rs` 与 `apps/desktop/tests/desktop_contracts.test.mjs` 分属 Rust/前端 owner 维护；必须继续消费同一共享 fixtures、拒 unknown/protocol/budget/security 回归，不能改 fixture 成假通过。
- Foundation 的空 ACL 和未注册命令是 deny-default 起点，Native 实现逐项绑定/授予；不得生成空成功 handler 后误报完成。Frontend 生产不得 import `docs/webui-demo/demo-data.js` 或静态合成交换。

## 最早双组件 rendezvous（不能后移）

Native 首先证明实际 Tauri/WKWebView 自定义媒体协议回调来源、Range、静音解码与暂停态跨段 seek，以及不向生产授予测试权限的真实 fixture driver。Frontend 同时提供单一桥、真实 DOM 操作 seam。若框架 API 与冻结假设不符，内部 Main 安排单 owner 修改；不得放宽到任意路径/窗口、通用shell、Chromium替代或假数据。

Native 必须提供 `packaging/desktop/{package,install,test-wkwebview}.sh` 实际入口及 `docs/desktop-native.md`；Frontend 必须提供 typecheck/test/build 与 `docs/desktop-ui.md`。Integration 才能更新总脚本并将 `docs/desktop-acceptance.md` 从 pending 改为带当前源证据的结果。

## 额外内部裁决

Main 批准受限 offline 写入所有者：`WriteAdmission.ownerGeneration` 可代表兼容 service 或显式初始化/隔离/已安全接管 profile 的锁定本地 owner；具体条件、token/profile/调用者/有效期绑定见[接口](../contracts/desktop/v1/README.md)。socket 不存在不是安全接管证明；legacy个人profile打开只读，不隐式迁移。offline安全配置保存不依赖Pi/Lark/模型，但录音与排任务仍需service。Native须增加双进程、旧daemon同时启动、重放与只读无迁移fixture。
