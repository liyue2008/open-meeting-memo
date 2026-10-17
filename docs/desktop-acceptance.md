# 桌面版本验收记录

**真实前端与原生组件已整合，完整产品验收尚未通过。** 生产校验器、最小关窗权限、私有网站数据隔离、媒体 revision 及受控关闭组件已整合至 `b8f5f9e`。当前实际 React/WKWebView 的146项矩阵通过：包含非空只读行动项、严格媒体断言和真实脏配置关窗取消/确认销毁及owned收尾。首次初始化/服务的真实React成功链、隔离的跨进程主题/侧栏保留及最终当前源全量门禁仍待直接证据，不能据此进入正式评审、用户目录安装或推送。下列状态区分组件证明与最终产品证明，不能用旧源结果、mock、编译成功或服务注册成功代替实际功能。

| 验收项 | 当前状态 | 最终需要的证据 |
|---|---|---|
| A1 工程/可重复入口 | 组件已整合；最终 clean source 待冻结 | 独立worktree/分支、Rust workspace＋npm双锁、清晰Swift原目录、统一脚本、clean committed source |
| A2 共享契约 | 36命令/100定义漂移通过；生产互操作修复与实际React库/版本读取通过 | 36命令、schema/Rust/TS无漂移、所有共享交换；真实桥适配器、协议/错误/预算/取消/CAS严格测试 |
| A3 Rust | 整合基线全量通过；修复后须重跑 | fmt/test --locked --all-targets/clippy -Dwarnings/build；desktop workspace同等门禁；持久并发/metadata/CRUD/import/版本/取消/config/文件安全fixture |
| A4 Swift | 整合基线 build-dev 与指定17+3 synthetic通过 | `build-dev.sh`＋AGENTS两组指定synthetic IPC ignored；不运行所有ignored或硬件 |
| A5 TypeScript/界面 | 基线101单时区；修复后102×UTC/洛杉矶通过；当前真实React146项通过，最终源须复验 | npm ci、严格类型、DOM交互、真实桥、日期/DST/分页/脏态/冲突、Markdown安全、明暗/侧栏全部设计 |
| A6 媒体/受托窗口 | 原生41项通过；最小权限、隔离、revision修复已整合，真实React metadata/段内跨段暂停定位及当前媒体定向负例通过 | 最早实际WKWebView来源/driver证明；verified同FD/hash/读锁、grant失效、Range/HEAD/206/416/ETag与并发/缓冲预算fixture |
| A7 全功能WKWebView | 当前146项含非空行动项及真实关闭通过；成功首用/服务的React链与跨进程偏好仍阻塞 | 私有profile＋真实new core/bridge＋fake helper/worker/Pi/notifier；实际窗口/交互/搜索/CRUD/版本/任务/配置/导出/明暗/侧栏；静音metadata/解码/暂停态段内跨段seek，禁止play |
| A8 release包 | 最终整合包pending；旧原生组件包不可替代 | 锁定aarch64 release app/dmg；macOS≥15.5、CLI/worker/helper/sherpa/codec资源闭包、嵌套身份/entitlements、内外ad-hoc签名、MIT/LGPL等完整许可与源码义务 |
| A9 实际安装/启动 | pending，未操作个人profile | metadata-only预检、非秘密元数据备份、用户目录真实生产包、保留录音/配置/认证/auto/consent；私有profile实际启动、窗口/正常桥验证、构建/安装来源hash及版本一致 |
| A10 独立评审 | pending | fresh只读正式评审无阻塞；修复后在最终当前源重跑所需门禁 |
| A11 SSH/PR | pending；当前禁止push | 最终clean commit经SSH推origin正式分支；远端full HEAD、PR API head、安装来源与源版本一致；创建PR不合并 |
| 跨版本/offline写权 | 原生并发/只读/generation组件回归通过；最终源待重验 | 旧daemon能力缺失、旧DB只读无迁移、全mutation提交前锁内generation重验、旧配置byte不变、offline双进程/跨profile重放/过期/旧daemon竞态拒写；显式切换诊断准确 |

## 当前整合证据与最终复验

私有证据根 `/private/tmp/omm-desktop-integration-8KrEI9`（0700）中的 `full-gate.log` / `full-gate.exit` 记录基线全量门禁 exit 0；`bridge-initial/bridge-result.json` 为41项原生WK fixture，`ok=true`、`playCalls=0`。`bridge-snapshot/bridge-result.json` 同时抓取真实React DOM，确认库校验失败；详细可复现命令与环境见该目录 `component-handoff.md`。原生fixture通过不能解除实际React阻塞。

前端修复 `ca5107f` 已摘取为 `1b6f52051e6bc32b5a58cc0402886b7bf6e20221`，`frontend-repair-gates.exit=0`：生成/strict TS/format、102测试×UTC/洛杉矶及生产build通过（含Vite实际输出的校验器回归）。未运行其附带WK脚本，不能据此关闭实际React阻塞。基线统一脚本只运行101个DOM测试一次；双时区/format/fixture-feature clippy是后续新增门禁，最终必须按当前脚本重跑。

原生隔离与最小权限三个修复已按原组件提交顺序整合。测试 feature 使用非持久 WK 网站数据仓库；原组件 A/B 独立仓库互不读取 sentinel 的实际证据已验证，生产持久化策略不变。不读取或清空个人网站数据。非持久仓库中的偏好保存及同窗口行为不证明生产跨进程持久化。

实际无媒体 React 矩阵根为 `/private/tmp/omm-integration-ui-without-media-arlcmv` 与 `/private/tmp/omm-integration-ui-without-media-srVeUp`：各 `bridge-result.json` 为118项、`ok=true`、`mediaNotTested=true`、`playCalls=0`，`command.exit`/`runtime.exit` 均0，`owned-process.json` 记录实际子进程正常退出且已等待，无超时。覆盖日期/排序/分页、版本、元数据 CAS、Trash恢复、重处理和取消、导入、fake helper同意与封存、配置冲突与原子保存、失败队列恢复、真实明暗/收缩/窄屏内容截图。完整hash清单见整合证据根 `without-media-handoff.json`；各 `post-run-source-freeze.json` 明确是事后来源冻结。

旧完整 driver 的 `retained trash detail` 失败是测试选择器错误：React 使用原生 `<dialog open>`，没有显式 `role=dialog`；旧 driver 查询不到 modal 后误点页头同名按钮。仅将整合 driver 选择器改为 `dialog[open],[role=dialog]`，未改组件，以上两次均严格验证真实 Trash/恢复。原失败保留。这两次运行未测试媒体，关闭确认不能用导航脏态提示代替。

媒体最小修复 `29a1f50` 已摘取为 `5c8c206`。随后实际执行 `OMM_DESKTOP_UI_EVIDENCE=/private/tmp/omm-integration-ui-media-fixed-Jm89LO bash scripts/test-desktop-ui-wkwebview.sh`，exit0：134项、`ok=true`、`playCalls=0`，无跳过媒体标记。严格 grant/manifest revision 相等；真实音频 metadata、第一段1.2秒和第二段1.5秒暂停定位、速度1.25及逐字稿timestamp均通过，连同Trash恢复、常规业务、真实内容索引查询和批量导出。7张实际WK内容截图中已直接查看录音上置、浅色展开和深色窄屏。owned PID83774正常exit0且wait，无超时；运行前后源码清单相等，`artifact-sha256.json` 保存实际driver/二进制/dist/结果/回执hash。原行动项检查仅证明区域存在且无checkbox：原seed缺少行动项标题，未证明非空列表；须补虚构seed标题并加强实际文本/列表断言，不改生产解析。不是最终clean来源；缺少真实关闭、成功首次初始化/服务和两次独立进程偏好保持的实际证据。最终须补这些场景，在最终当前clean提交重跑全量门禁并重新构建整合release。

### 非空行动项与真实关闭定向续验

已采用native close `f750f5b` 与生命周期/虚构种子标题 `39aaf2a`，在整合HEAD `b8f5f9e36a90a45adbce1e766533526b8548b049`＋明确的整合fixture WIP上实际执行：

```sh
OMM_DESKTOP_UI_EVIDENCE=/private/tmp/omm-integration-close-actions-xA9HjC \
bash scripts/test-desktop-ui-wkwebview.sh
```

`wrapper.exit`、`command.exit`、`runtime.exit`均0；146项、`ok=true`、`playCalls=0`。这次实际要求行动区含 `Read-only synthetic action`、至少1个列表项，且行动区和artifact全区均无交互checkbox；生产解析未改，旧134不追认为该非空证明。原媒体/完整常规业务断言保留。

末尾实际修改配置形成草稿，由可信Rust入口两次真实请求关闭：第一次点击React“留在设置”后，同窗口/草稿保留，磁盘revision及配置值未变；第二次提示后先arm并收到匹配回执，再点真实React“放弃并离开”。`close-before-destroy.json`保存真实提示DOM及mainAlive，最终 `nativeClose` 的destroyedEvent/mainAbsent/readersReleased/shutdownAcknowledged/captureExitSuccess/captureChildWaited全true。owned PID78734正常exit0/waited，无超时，后续精确PID查询无匹配。关窗后实际配置文件SHA256仍等于草稿前revision `55e86970f2a51fd928203b7a9c707dcdb9903a563a025262b7dc38583ea3b548`，确认放弃没有保存。

运行前后269条源码清单完全相等；driver、实际dist/二进制、结果及回执hash见 `artifact-sha256.json`。`verification.json`补充真实关闭前文件、8张截图hash及关窗后配置hash复核。新增 `dirty-close-confirmation.png` 已直接查看；录音上置截图也已直接查看。为遵守native arm请求64KiB上限，仅将诊断轮询聚合为命令计数、截图DOM文字改为元数据；未删业务断言，截图文件和native关闭前DOM独立保留。

该运行不是最终clean release；真实首次启用/服务React操作仍待受限driver入口及当前候选包资源复验。偏好仍为nonpersistent同窗口证据，未做两进程保持或更改全局OS主题。此定向块完成不代表完整Goal候选就绪。

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
