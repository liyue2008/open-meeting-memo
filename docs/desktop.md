# 桌面 App 开发

当前新增 macOS Apple Silicon、macOS 15.5+ 的 Tauri 2 桌面工程。Foundation 只建立隔离工程、类型契约与编译基线；**它还不是可用桌面产品，不能代替后续完整实现、评审和安装验收**。桌面内嵌系统 WKWebView，不开放 HTTP 产品服务；浏览器版 WebUI、Windows/Linux、MCP 仍是后续目标。

## 目录和单一职责

| 路径 | 负责什么 |
|---|---|
| `Cargo.toml`、`src/{core,service,cli,adapters}` | 保留现有 Rust 包、CLI/daemon/worker；核心继续是持久状态/产物的唯一真源 |
| `src/application` | App 与 CLI 共用的类型/用例入口；不把业务复制进 Tauri handler |
| `apps/desktop/src` | React/严格 TypeScript 界面、单一 typed bridge；不持有路径/凭据/任意 shell 权限 |
| `apps/desktop/src-tauri` | Tauri 2 薄 Rust 宿主、受托窗口命令与媒体协议；加入根 Cargo workspace，不另建 Cargo.lock |
| `native/yulu-capture` | 原 Swift/Yulu 录音内核与现有构建入口；不为目录美观移动或改写 PCM/同步/WAV/恢复代码 |
| `contracts/desktop/v1` | 版本化 schema、合成交换、生成命令表及冻结安全语义 |
| `scripts/*-desktop.sh` | 全项目统一 dev/test/build/package/install 门面，委托现有 Swift/native 构建入口 |
| `packaging/desktop` | 后续 native owner 提供资源闭包、签名、包检查与用户级安装脚本 |
| `docs/webui-demo`、`docs/assets` | 已确认静态设计及独立 Logo 的精确导入；是设计材料，不是生产数据源 |

保留 root Rust crate 和 Swift 原目录，不重写已验证的安全/原生边界。Rust、TypeScript、Swift 各自使用成熟工具链，只提交根 Cargo.lock 与 `apps/desktop/package-lock.json`。Node 是前端开发依赖；外部 Pi 自身的 Node 需求须独立诊断，不能把 Vite 放进生产后台。

## 可重复入口

需要现有 Apple Silicon macOS/Xcode/Rust、Node 22.19–22.x、npm 11.x（锁由 npm 11.21.0 生成）。脚本不安装系统工具、模型、用户服务或权限。首次明确安装项目依赖：

```sh
cd apps/desktop && npm ci && cd ../..
bash scripts/dev-desktop.sh
bash scripts/test-desktop.sh --foundation  # 当前组件门禁，不是全产品验收
bash scripts/test-desktop.sh               # Rust/TS/指定 Swift fixture；后续继续补桌面验收
bash scripts/build-desktop.sh
bash scripts/build-desktop.sh --release
bash scripts/package-desktop.sh
bash scripts/install-desktop.sh --check
bash scripts/install-desktop.sh --from /absolute/path/to/verified/OMM.app
```

脚本接受参数见脚本源；统一执行 `--locked` 和契约漂移检查，用 `npm ci --ignore-scripts` 仅安装锁定的项目级依赖。开发服务器只监听127.0.0.1；release CSP 不允许开发地址。`bundle.active=false` 与空权限是有意的 Foundation 默认拒绝，主功能尚未接线。package/install 在 native 实现并通过安全/资源检查前明确失败，不生成假发行包或安装空壳。安装必须仅当前用户、检查旧元数据、保留配置/录音/认证/auto/consent；不复制秘密备份、不打断录制、不盲杀/擅重启个人 daemon。可以经过核验后 versioned side-by-side 安装生产包并用私有 profile 实际启动；个人服务切换另行准确报告。

## 已确定的完整功能范围

- 会议库真实搜索、状态/标签/日期筛选、排序、分页与批量操作。默认最近7个自然日，支持今天、周一开始本周、最近3/7天，覆盖本地时区/夏令时。
- 安全创建/原生导入/录制、metadata 版本比较写入、保留式删除与 Trash 恢复。删除不物理清音频/版本/检查点。
- 详情顶部真实 mix 分段音频、时间定位、逐字稿/纪要/只读行动项、复制和受限导出；失败重处理仍能读取旧有效产物与版本。
- 持久队列、有限重试、真实取消/等待 owned children、失败恢复；配置全白名单原子保存、脏态和外部写冲突；诊断/帮助及 CLI 合理配套。
- 批准设计的浅色、柔和炭灰深色与鲜明青绿强调；实时随 OS＋手动覆盖；统一图标、持久侧栏收缩、双色自绘 omm Logo。会议内容不进入浏览器持久存储。

生产不使用静态 demo-data、硬编码成功、任意路径/通用 shell 或认证继承。会议 Markdown 禁 HTML/脚本/远程资源。真实音频仅用户点击播放；本开发验收**禁止播放、录音硬件、真实通知、个人会议数据/认证及真实模型调用**。

## 共享契约和兼容性

参见 [冻结接口与安全语义](../contracts/desktop/v1/README.md)。先 read-only 检查 profile 和版本化 service capability，不能打开即迁移旧数据库。旧/未知 daemon 无写权；写入 admission 绑定规范 profile、所有者 generation、可信调用者和有效期，提交前持锁再检查。受限 offline admission 只允许显式初始化/隔离/已安全接管的新 profile 做安全配置保存，不伪装录音/任务已可运行。现有 legacy profile 不被隐式迁移。

native/frontend 从同一 Foundation 本地提交分别分叉，组件独占规则见 [lane board](desktop-lanes.md)。完成组件提交后才由 integration-only 汇合；[验收矩阵](desktop-acceptance.md) 全通过、fresh 独立正式评审无阻塞、实际 release 用户级安装验证后，才 SSH push 并创建 PR，不自动合并。
