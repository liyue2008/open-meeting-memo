# 桌面 App 开发

当前 macOS Apple Silicon、macOS 15.5+ 的 Tauri 2 桌面分支已合入真实前端与原生组件，正在执行整合验收。**组件测试不等于产品验收；完整真实界面门禁、独立评审与最终用户安装状态见[验收记录](desktop-acceptance.md)**。桌面内嵌系统 WKWebView，不开放 HTTP 产品服务；浏览器版 WebUI、Windows/Linux、MCP 仍是后续目标。

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
| `packaging/desktop` | 资源闭包、签名、包检查、受控 WKWebView 与用户级安装脚本 |
| `docs/webui-demo`、`docs/assets` | 已确认静态设计及独立 Logo 的精确导入；是设计材料，不是生产数据源 |

保留 root Rust crate 和 Swift 原目录，不重写已验证的安全/原生边界。Rust、TypeScript、Swift 各自使用成熟工具链，只提交根 Cargo.lock 与 `apps/desktop/package-lock.json`。Node 是前端开发依赖；外部 Pi 自身的 Node 需求须独立诊断，不能把 Vite 放进生产后台。

## 可重复入口

需要现有 Apple Silicon macOS/Xcode/Rust、Node 22.19–22.x、npm 11.x（锁由 npm 11.21.0 生成）。脚本不安装系统工具、模型、用户服务或权限。首次明确安装项目依赖（锁中的官方包下载地址使用公开 `registry.npmjs.org`，不要求项目私有镜像或认证；版本/依赖图/integrity保持固定）：

```sh
cd apps/desktop && npm ci && cd ../..
bash scripts/dev-desktop.sh
bash scripts/test-desktop.sh --foundation  # 有限组件门禁，不是全产品验收
bash scripts/test-desktop.sh               # 全量 Rust/TS/指定 Swift fixture
bash scripts/test-desktop-ui-wkwebview.sh  # 单独复验实际React交互；不重跑Rust/Swift单测
bash scripts/test-desktop-preferences-wkwebview.sh  # UUID隔离→两真实React进程→精确清理
bash scripts/build-desktop.sh
bash scripts/build-desktop.sh --release
bash scripts/package-desktop.sh          # 先提交并保持clean，资源来自同一revision
export OMM_LIFECYCLE_RESOURCE_APP="$PWD/target/desktop-package/$(git rev-parse HEAD)/Open Meeting Memo.app"
bash scripts/test-desktop.sh --wkwebview # 全量加原生/React/UUID两进程与首用服务正负链
bash scripts/test-desktop-lifecycle-wkwebview.sh # 单独真实React及原包CLI；仍要求上述当前资源包
bash scripts/install-desktop.sh --check
bash scripts/install-desktop.sh --from /absolute/path/to/verified/OMM.app
```

脚本接受参数见脚本源；统一执行 `--locked` 和契约漂移检查。统一依赖准备使用 `npm ci --ignore-scripts`，具体包装和WK fixture再按锁执行项目级 `npm ci`/build；不安装系统或全局工具。开发服务器只监听127.0.0.1；release CSP 不允许开发地址。`bundle.active=false` 禁止 Tauri 自行发布不完整资源包；实际包装由 `packaging/desktop/package.sh` 组装并核验自有运行时、资源、许可证与内外签名。36 个业务命令逐项授予可信 main 窗口，没有任意文件/路径/shell 权限。package 要求已提交的干净源码，结果按完整 revision 放在 `target/desktop-package/<revision>/`。安装必须仅当前用户、检查旧元数据、保留配置/录音/认证/auto/consent；不复制秘密备份、不打断录制、不盲杀/擅重启个人 daemon。可以经过核验后 versioned side-by-side 安装生产包并用私有 profile 实际启动；个人服务切换另行准确报告。

新冷首次/partial入口使用原样 `apps/desktop/src-tauri/fixtures/review-probe.js` 和 `OMM_NATIVE_REVIEW_FIXTURE=cold-recording|partial-transcript`，默认不设置持久store清单。前者在真实同意/开始前无peer/socket，经生产ensure_helper实际spawn，停止封存后restore/quit/wait；后者只通过inspect.scenario取真实parent，受限advance/finalize推进实际段commit和全篇aggregate，UI必须显示部分标记/更新/终稿，旧cursor冲突独立验证。没有新的生产命令或权限，reader泄漏在fixture teardown前就必须失败；不能以兜底清理抹去失败。

## 已确定的完整功能范围

- 会议库真实搜索、状态/标签/日期筛选、排序、分页与批量操作。默认最近7个自然日，支持今天、周一开始本周、最近3/7天，覆盖本地时区/夏令时。
- 安全创建/原生导入/录制、metadata 版本比较写入、保留式删除与 Trash 恢复。删除不物理清音频/版本/检查点。
- 详情顶部真实 mix 分段音频、时间定位、逐字稿/纪要/只读行动项、复制和受限导出；失败重处理仍能读取旧有效产物与版本。
- 持久队列、有限重试、真实取消/等待 owned children、失败恢复；配置全白名单原子保存、脏态和外部写冲突；诊断/帮助及 CLI 合理配套。
- 批准设计的浅色、柔和炭灰深色与鲜明青绿强调；实时随 OS＋手动覆盖；统一图标、持久侧栏收缩、双色自绘 omm Logo。会议内容不进入浏览器持久存储。

生产不使用静态 demo-data、硬编码成功、任意路径/通用 shell 或认证继承。会议 Markdown 禁 HTML/脚本/远程资源。真实音频保留原生controls及明确的“播放录音/暂停录音”按钮；grant/metadata未就绪时按钮禁用，仅用户明确点击播放，换段/定位不自动播放。播放器事件接线的单测使用mock，真实WK只验证可见入口、metadata及暂停定位，不验证可听发声。本开发验收**禁止播放、录音硬件、真实通知、个人会议数据/认证及真实模型调用**。

## 共享契约和兼容性

参见 [冻结接口与安全语义](../contracts/desktop/v1/README.md)。先 read-only 检查 profile 和版本化 service capability，不能打开即迁移旧数据库。旧/未知 daemon 无写权；写入 admission 绑定规范 profile、所有者 generation、可信调用者和有效期，提交前持锁再检查。受限 offline admission 只允许显式初始化/隔离/已安全接管的新 profile 做安全配置保存，不伪装录音/任务已可运行。现有 legacy profile 不被隐式迁移。

native/frontend 从同一 Foundation 本地提交分别分叉，已按等价契约去重汇合；组件独占与修复退回原 owner 的规则见 [lane board](desktop-lanes.md)。完成组件提交后才由 integration-only 汇合；[验收矩阵](desktop-acceptance.md) 全通过、fresh 独立正式评审无阻塞、实际 release 用户级安装验证后，才 SSH push 并创建 PR，不自动合并。

## 统一门禁与证据边界

需求/CLI用例与源码命名测试的逐项对应见[能力映射](desktop-capability-matrix.md)。完整测试入口会执行私有 wrapper 参数回归、锁定 npm 安装、36命令/100定义共享契约、格式/严格类型、UTC 与 America/Los_Angeles 两个时区的 DOM 测试、生产构建、root 与 workspace 的 Rust fmt/test/clippy/build、fixture-feature clippy，以及指定的 Swift synthetic IPC 17+3 项。`--foundation` 只运行有限组件子集；`--wkwebview` 额外启动私有合成 profile 的真实 macOS WKWebView，不能以 Chromium 或 DOM mocks 代替。原生/ACL、F2/F5队列/内容、F7明暗/播放入口、cold-recording及partial-transcript各进程均保存独立源码前后清单和实际宿主/dist/driver/结果/owned hash。

实际 React fixture 使用 `scripts/fixtures/desktop-ui-probe.js` 驱动真实控件，保持生产 Tauri transport 不变。独立入口只接受全新空的私有 `/private/tmp` 证据根（可由 `OMM_DESKTOP_UI_EVIDENCE` 指定），记录运行前后源码 SHA256、实际 driver/dist/二进制/结果 hash 和 owned 进程退出回执；源码变化、业务断言失败、播放或超时均失败。默认WKWebView fixture使用非持久网站数据存储，不复用个人App缓存/偏好。独立偏好入口先执行native A/B隔离证明，再从工具分配的新owned UUID运行两个独立React进程，第一进程通过真实主题/侧栏控件写入，第二进程在任何写/清除前核对初始dark/collapsed，均经合法dirty close退出；最后只精确清理分配的UUID。入口记录每阶段源码/实际binary/dist/hash/owned回执，可用 `OMM_DESKTOP_PREFERENCES_EVIDENCE` 指定全新canonical私有证据根（最长55字节，避免Unix socket路径超限）。不自造UUID/nonce、不clear localStorage、不读取personal/default store；这不是个人存量偏好的兼容性证明，也不改全局OS主题。

真实React矩阵要求非空只读行动项（实际文本及列表、无checkbox），末尾经受限Rust入口请求真实关闭：取消保留草稿且磁盘未保存，确认前保存证据并arm，点击原React放弃按钮后，由native记录真实destroy、reader释放、服务关闭及owned helper等待。驱动不直接调用destroy/emit；证据入口独立核对关闭前记录与最终回执。native arm请求上限为64KiB，因此仅聚合诊断轮询、保留截图元数据，完整截图和关闭前DOM仍独立保存。

完整媒体路径使用 verified descriptor/hash 和短效 grant；fixture 只做无输出节点的静音解码、metadata 与暂停定位，`playCalls` 必须为0。生产播放仅由用户控制。真实权限、硬件采集、语音识别质量、个人 Pi 认证、可见通知及长会议质量不由这些 fixture 证明。

首用服务门禁通过 `OMM_LIFECYCLE_RESOURCE_APP` 显式绑定当前clean revision的资源包，拒绝旧包来源。自有React驱动经受限canonical/owned0600 UTF8<128KiB入口原样执行：真实初始化/取消、60秒自然到期与重检、旧plan真正stale、held归属不明和录音/未封存拒绝、恢复后启动/切换/停止。另跑默认原生4case；两阶段原包CLI/manifest前后字节不变，实际CLI PID路径经身份校验。只替换私有fixture宿主并外层ad-hoc重签，不能把测试宿主说成原生产宿主。`source-before/after`、实际driver/宿主/dist/资源及owned回执由独立入口保存；失败保留并由native自动restore/shutdown/wait。它仍不安装LaunchAgent，不请求权限或调用真实模型。

发布流程是：当前源码全量验证 → fresh 只读正式评审 → 最终 clean commit 构建并验证资源/signature/license/source hash → 显式用户目录 side-by-side 安装并用私有 profile 实际启动 → 既有 SSH origin push 与 GitHub PR（不 merge）。整合 owner 不提前安装或推送；组件包中旧 Foundation 界面不能冒充最终产品包。

生产App的普通启动仍保留持久主题/侧栏偏好。只有明确传`--ephemeral-webview`才在创建main前选择非持久WebKit，`--webview-store-diagnostics`配合它输出实际对象`persistent=false`的有界非内容诊断；不开放fixture控制器、不授予初始化或服务写权限。隔离验收使用真实已安装生产字节、清继承环境与全新私有HOME/所有XDG根，具体命令见[安装指南](install.md#桌面-app-状态)。不能用fixture或仅HOME/XDG替代生产WebKit隔离，也不打开个人/default store作对照。
