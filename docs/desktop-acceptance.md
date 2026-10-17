# 桌面版本验收记录

**真实前端与原生组件已整合，完整产品验收尚未通过。** 当前基线 `8639079296dc26f37791109a4df1b8ee23621af6` 已运行全量组件门禁，但真实 React/WKWebView 暴露了生产校验器互操作及关窗监听权限问题，正在由原组件 owner 隔离修复。修复前不进入正式评审、用户目录安装或推送。下列状态区分组件证明与最终产品证明，不能用旧源结果、mock、编译成功或服务注册成功代替实际功能。

| 验收项 | 当前状态 | 最终需要的证据 |
|---|---|---|
| A1 工程/可重复入口 | 组件已整合；最终 clean source 待冻结 | 独立worktree/分支、Rust workspace＋npm双锁、清晰Swift原目录、统一脚本、clean committed source |
| A2 共享契约 | 36命令/100定义漂移检查通过；真实 React 互操作修复中 | 36命令、schema/Rust/TS无漂移、所有共享交换；真实桥适配器、协议/错误/预算/取消/CAS严格测试 |
| A3 Rust | 整合基线全量通过；修复后须重跑 | fmt/test --locked --all-targets/clippy -Dwarnings/build；desktop workspace同等门禁；持久并发/metadata/CRUD/import/版本/取消/config/文件安全fixture |
| A4 Swift | 整合基线 build-dev 与指定17+3 synthetic通过 | `build-dev.sh`＋AGENTS两组指定synthetic IPC ignored；不运行所有ignored或硬件 |
| A5 TypeScript/界面 | 101 DOM测试在UTC/洛杉矶通过；真实WK功能未通过 | npm ci、严格类型、DOM交互、真实桥、日期/DST/分页/脏态/冲突、Markdown安全、明暗/侧栏全部设计 |
| A6 媒体/受托窗口 | 41项原生WK fixture通过；关窗最小权限修复中 | 最早实际WKWebView来源/driver证明；verified同FD/hash/读锁、grant失效、Range/HEAD/206/416/ETag与并发/缓冲预算fixture |
| A7 全功能WKWebView | 阻塞：实际React库/版本读取失败，关窗监听未获最小权限 | 私有profile＋真实new core/bridge＋fake helper/worker/Pi/notifier；实际窗口/交互/搜索/CRUD/版本/任务/配置/导出/明暗/侧栏；静音metadata/解码/暂停态段内跨段seek，禁止play |
| A8 release包 | 最终整合包pending；旧原生组件包不可替代 | 锁定aarch64 release app/dmg；macOS≥15.5、CLI/worker/helper/sherpa/codec资源闭包、嵌套身份/entitlements、内外ad-hoc签名、MIT/LGPL等完整许可与源码义务 |
| A9 实际安装/启动 | pending，未操作个人profile | metadata-only预检、非秘密元数据备份、用户目录真实生产包、保留录音/配置/认证/auto/consent；私有profile实际启动、窗口/正常桥验证、构建/安装来源hash及版本一致 |
| A10 独立评审 | pending | fresh只读正式评审无阻塞；修复后在最终当前源重跑所需门禁 |
| A11 SSH/PR | pending；当前禁止push | 最终clean commit经SSH推origin正式分支；远端full HEAD、PR API head、安装来源与源版本一致；创建PR不合并 |
| 跨版本/offline写权 | 原生并发/只读/generation组件回归通过；最终源待重验 | 旧daemon能力缺失、旧DB只读无迁移、全mutation提交前锁内generation重验、旧配置byte不变、offline双进程/跨profile重放/过期/旧daemon竞态拒写；显式切换诊断准确 |

## 当前整合证据与最终复验

私有证据根 `/private/tmp/omm-desktop-integration-8KrEI9`（0700）中的 `full-gate.log` / `full-gate.exit` 记录基线全量门禁 exit 0；`bridge-initial/bridge-result.json` 为41项原生WK fixture，`ok=true`、`playCalls=0`。`bridge-snapshot/bridge-result.json` 同时抓取真实React DOM，确认库校验失败；详细可复现命令与环境见该目录 `component-handoff.md`。原生fixture通过不能解除实际React阻塞。

修复交接后必须在新整合提交重跑完整门禁和全业务真实WK测试，并在本页替换为最终当前源码与工件证据。

WK测试隔离待证实：只将HOME/XDG指向私有目录不足以证明macOS网站数据仓库隔离。原生owner须提供test-only非持久仓库或明确独立受控仓库及A/B互不读取sentinel的证据，生产持久外观行为不变。在此之前暂停写主题/侧栏网站偏好的WK测试；不读取或清空既有个人网站数据。非持久仓库中同窗口reload不证明生产跨进程持久化。

依赖审计的外部限制：当前 `npm audit` 未取得报告。证据根中的 `npm-audit-current.exit`、`npm-audit-official-current.exit`、`npm-audit-system-ca.exit` 均为1：项目镜像审计API返回400；官方端点先遇证书链验证失败，使用现有系统信任后返回网络策略拦截页面。未关闭TLS、绕过网络策略或改动依赖。这是未完成的审计，不是“无漏洞”，也不是已发现漏洞。已知Vitest更新提示由原前端owner核对适用公告；正式评审必须考虑该验证限制。

以下Foundation入口只是有限组件检查：

```sh
bash scripts/test-desktop.sh --foundation
cargo clippy --locked --workspace --all-targets -- -D warnings
bash scripts/build-desktop.sh
node scripts/generate-desktop-contracts.mjs --check
git diff --check
git diff --cached --name-only # 交接提交后必须为空
```

静态Demo/Logo导入须逐字节匹配批准的13文件hash，原设计树保持不变。README只局部增加Logo并准确链接桌面状态。根Cargo.lock保留所有原package版本；新依赖只为Tauri必要闭包。生产目录不得使用demo/mock逻辑、未授权命令或任意asset路径协议。

## 测试安全边界

不播放任何音频，不启动录音硬件，不发真实通知，不读个人会议/认证，不调用真实云模型，不下载权重。使用0700私有临时根、合成静音WAV、fake进程和受控C API。截图/长日志/生成bundle不入git。fixture不能证明系统真实授权、会议检测准确度、语音识别质量、个人Pi认证、可听播放或长会议质量。ad-hoc开发签名不能称为公证。
