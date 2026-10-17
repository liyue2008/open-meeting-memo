# 桌面版本验收记录

**首轮正式业务与安全评审均要求修复，当前不允许安装或发布。** 已有候选门禁不能覆盖正式评审发现；UI可读性、明确播放入口与公开依赖锁修复已完成定向验证。后端正式修复`dde5269`、`2c4d5f9`已摘取为`ae4c608`、`b190062`，受控Rust回归与F2/F5实际UI续验通过；冷首次录音/partial的真实交互入口尚待独立native测试增补，不能用预启动/完整成功seed冒充。所有入口齐备后须重跑当前源门禁并由新的独立席位复审。 既有146项React/WK矩阵包含非空只读行动项、严格媒体和真实脏配置关窗；测试专属UUID两进程偏好及真实React首用/服务入口也已接线。后续held busy/unknown/stale受限控制和原包CLI字节保留已采用native `1d27d271`，不改生产schema/权限。每次候选的精确commit、当前源完整运行结果、实际exit/owned回执及资源hash，必须由随候选交接的证据矩阵确认；本页的历史通过不能追认为新源通过。A9–A11仍须Main另行编排，不因组件通过或候选就绪而自动完成。

| 验收项 | 当前状态 | 最终需要的证据 |
|---|---|---|
| A1 工程/可重复入口 | 组件与统一门禁齐备；每次clean来源以候选证据矩阵冻结 | 独立worktree/分支、Rust workspace＋npm双锁、清晰Swift原目录、统一脚本、clean committed source |
| A2 共享契约 | 36命令/100定义漂移通过；生产互操作修复与实际React库/版本读取通过 | 36命令、schema/Rust/TS无漂移、所有共享交换；真实桥适配器、协议/错误/预算/取消/CAS严格测试 |
| A3 Rust | 整合基线全量通过；最新当前源结果见候选证据矩阵 | fmt/test --locked --all-targets/clippy -Dwarnings/build；desktop workspace同等门禁；持久并发/metadata/CRUD/import/版本/取消/config/文件安全fixture |
| A4 Swift | 整合基线 build-dev 与指定17+3 synthetic通过 | `build-dev.sh`＋AGENTS两组指定synthetic IPC ignored；不运行所有ignored或硬件 |
| A5 TypeScript/界面 | 历史146及定向19项通过；当前组合109×UTC/洛杉矶和F2/F5实际UI23项通过，最终全矩阵须复验 | npm ci、严格类型、DOM交互、真实桥、日期/DST/分页/脏态/冲突、Markdown安全、明暗/侧栏全部设计 |
| A6 媒体/受托窗口 | 原生41项通过；最小权限、隔离、revision修复已整合，真实React metadata/段内跨段暂停定位及当前媒体定向负例通过 | 最早实际WKWebView来源/driver证明；verified同FD/hash/读锁、grant失效、Range/HEAD/206/416/ETag与并发/缓冲预算fixture |
| A7 全功能WKWebView | 既有146项及fab32首用/偏好准备通过；完整React held负例/原包CLI入口已接，当前组合结果须另行实证 | 私有profile＋真实new core/bridge＋fake helper/worker/Pi/notifier；实际窗口/交互/搜索/CRUD/版本/任务/配置/导出/明暗/侧栏；静音metadata/解码/暂停态段内跨段seek，禁止play |
| A8 release包 | fab32准备包2031资源验证通过；每次最终包须绑定当前clean revision及原CLI字节，旧包不能替代 | 锁定aarch64 release app/dmg；macOS≥15.5、CLI/worker/helper/sherpa/codec资源闭包、嵌套身份/entitlements、内外ad-hoc签名、MIT/LGPL等完整许可与源码义务 |
| A9 实际安装/启动 | pending，未操作个人profile | metadata-only预检、非秘密元数据备份、用户目录真实生产包、保留录音/配置/认证/auto/consent；私有profile实际启动、窗口/正常桥验证、构建/安装来源hash及版本一致 |
| A10 独立评审 | 首轮needs_changes；修复组合及新fresh席复审pending | fresh只读正式评审无阻塞；修复后在最终当前源重跑所需门禁 |
| A11 SSH/PR | pending；当前禁止push | 最终clean commit经SSH推origin正式分支；远端full HEAD、PR API head、安装来源与源版本一致；创建PR不合并 |
| 跨版本/offline写权 | 原生并发/只读/generation组件回归通过；最终源待重验 | 旧daemon能力缺失、旧DB只读无迁移、全mutation提交前锁内generation重验、旧配置byte不变、offline双进程/跨profile重放/过期/旧daemon竞态拒写；显式切换诊断准确 |

## 首轮正式评审后的定向修复

F7只调整首次/归属不明服务横幅的专用入口：明确前景/背景并关闭系统按钮外观；正常、hover及focus使用同一前景/背景配对，仅改变边框。实际WK明暗计算对比度及稳定截图由定向证据记录，DOM样式断言不是实际鼠标hover证明。生产录音保留原生audio controls，另有明确的“播放录音/暂停录音”按钮：grant与metadata就绪前禁用，仅用户明确点击播放；定位、换段、恢复窗口不自动播放。播放事件接线仅使用单元测试mock，真实WK保持0次play，不能声称已验证可听回放。

npm锁的下载地址由私有镜像规范为官方公开registry，保留全部版本、依赖图及integrity。隔离复制中的完整性缓存离线安装验证与公开在线安装是两种证据；后者和完整audit仍受网络/TLS策略限制。没有改全局npm配置、认证、证书、TLS或依赖版本。冷首次录音、导入/中断、互斥、partial读取、idle shutdown及content检索的生产修复已组合，当前受控Rust回归83项和desktop fixture-feature7项通过；83项中另6个ignored未在该命令运行，不能追认为已跑完整Swift。F2/F5已补实际UI23项（仅标题content搜索、Trash后retry/requeue禁用及恢复）。预启动fixture不证明冷首次启动，已完成产物不证明partial读取；两项真实交互仍待新测试seam。UI早期busy拒绝不是F4最终执行点竞态证明，后者由Core受控service/claim/reader回归单独提供；导入中断故障注入也不能替代UI闭环。

## 正式修复组合阶段A（尚未新候选）

当前生产修复已按两条immutable提交依序摘取，源与`2c4d5f9`的17条变更路径逐字节一致。`/private/tmp/omm-review-integrate-hxRI0P/stage-a-gates.log`与`.exit`记录fmt、34 core＋8 imports＋19 desktop＋22 segments共83回归、7个desktop feature测试、合同/validator漂移、strict TS、109测试双时区和生产build全部0；不是最终当前源全量。

`scripts/fixtures/desktop-review-guards-probe.js`已接入统一`--wkwebview`。实际定向根`/private/tmp/omm-rg-VXDEyk`：23项/ok=true/playCalls=0，owned PID38348 exit0/waited无超时。通过真实UI给failed/cancelled两会写唯一标题，确认无成功内容仍能在content范围仅凭title匹配；真实Trash后队列retry/requeue均禁用，恢复后只恢复原本合法操作。raw调用仅独立读取DTO，不代替UI写操作；状态/阶段/虚构worker失败的真实语义按冻结契约保留。该次记录immutable clean来源及实际driver/binary/dist，未单独建立运行前清单；最后统一验收必须有完整before/after。

在Main批准的命令环境下，原脚本真实`npm ci --no-audit --no-fund`可使用既有已验证tarball缓存及offline=true；先仅将项目node_modules备份到0700私有根，实际版本/完整性图前后比较，失败保留并恢复旧依赖。没有跳ci/改TLS、代理或个人配置。npm11既有allowScripts提示要保留，不能因没传ignore-scripts就声称所有postinstall已执行；no-audit也不是完整漏洞审计。此前私有跳ci harness仅属旧定向证据，不能沿用为最终原门禁通过。

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

### 历史fab32来源资源与准备证据（不追认后续候选）

仅摘native `1cb3edf` 为 `fab32d7ccd5e8836d0e3fda3543080642cdde12d`；36schema/ACL/CSP和production持久策略未变。clean fab32实际打包exit0：`target/desktop-package/fab32d7ccd5e8836d0e3fda3543080642cdde12d/Open Meeting Memo.app`，2031资源verified=true，包含当前真实UI/CLI/worker/helper/sherpa/codec和许可证，owned包装进程已wait。完整证据 `/private/tmp/omm-candidate-current-kMHzGp`。这不是最终新增fixture/文档提交的release。

该App实际作为 `OMM_LIFECYCLE_RESOURCE_APP`，受限as-is自有React driver运行于 `/private/tmp/omm-lifecycle-proof-T5aqvI`：run/owned wrapper exit0，36项真实按钮/取消初始化/初始化/自然到期拒绝和重检/启动/安全切换/停止通过，playCalls=0。两CLI40303/40665及capture40419已owned收尾；provenance明确sourceRevision=fab32、资源2031及实际hash，不是旧f139。必须注意：当时脚本重新编译并替换了私有副本CLI，原包CLI为bf7fd49b...，实际执行为e91c9258...；这是当前源码重编译CLI的准备证明，不能称原包CLI字节已验。Main已批准同native改测试入口保留原包CLI并记录source/staged/运行后hash相等；最终须同源新包重跑。busy/unknown/stale当时只有native原4case直接证据，不能替代真实React负例；另已批准同native补feature-only held控制，Root不自行改组件或竞态测试。

专属存储新入口 `bash scripts/test-desktop-preferences-wkwebview.sh` 已实际exit0，证据 `/private/tmp/omm-pref-Cm4kfJ`：先A/B隔离proof、再工具分配新UUID，两个实际React进程第一次真实写dark/collapsed，第二次任何偏好写/清除前初始还原，两次真实dirty close/native销毁及owned收尾通过，最终exact cleanup removed=true。每阶段在cleanup重建前保存实际binary/dist/driver/result/owned hash，源码前后相等；运行时fab32加明确的自有fixture WIP，不冒称后续最终提交已全量重跑。

准备期间保留两个driver失败：一个旧label选择器漏aria-label；一个证据路径太长导致Unix socket超限。均只修自有driver/改短私有根，失败进程已wait且分配UUID精确removed，没有产品改动或断言放宽。现入口先拒绝过长根，完整旧失败及新实际0分别保存。

依赖审计的外部限制：当前 `npm audit` 未取得报告。证据根中的 `npm-audit-current.exit`、`npm-audit-official-current.exit`、`npm-audit-system-ca.exit` 均为1：项目镜像审计API返回400；官方端点先遇证书链验证失败，使用现有系统信任后返回网络策略拦截页面。未关闭TLS、绕过网络策略或改动依赖。这是未完成的审计，不是“无漏洞”，也不是已发现漏洞。本阶段通过官方公开GitHub Advisory API取回 [GHSA-9crc-q9x8-hgqq](https://github.com/advisories/GHSA-9crc-q9x8-hgqq)：公告修复版本为1.6.1/2.1.9/3.0.5，当前锁定Vitest5.0.5不在该公告受影响范围。证据 `vitest-advisory.json/.exit` 在当前准备根；没有关闭TLS或擅改依赖，也不把单个公告核对充作完整审计。正式评审必须收到仍失败的npm audit及网络限制。

### 当前完整候选入口与实际结果要求

已采用native held控制与CLI保留补丁；自有 `scripts/fixtures/desktop-lifecycle-probe.js` 必须通过真实React按钮证明初始化/取消、自然到期/重检、已展示旧plan的真正generation冲突、归属不明和录音/未封存拒绝，以及restore后正常启动/切换/停止。默认原生4case另跑，不能用raw结果替React。受限控制仅在owned测试main，single-held90秒/32次且finish/失败自动restore及wait，不放宽生产权限。

在当前源码提交且clean后 `bash scripts/package-desktop.sh`，显式设置该revision App的 `OMM_LIFECYCLE_RESOURCE_APP`，再 `bash scripts/test-desktop.sh --wkwebview`。其中 `scripts/test-desktop-lifecycle-wkwebview.sh` 拒绝旧包revision，保留原CLI和manifest字节、实际运行PID路径及每阶段宿主/dist/driver/hash；只有测试宿主替换并外层ad-hoc重签。前后全源码清单必须相等。其他真实UI/媒体/关闭和专属UUID两进程入口仍独立运行，成功不能由组件或编译代替。完整当前源/退出/覆盖/未验边界及旧失败由候选交接报告逐项列出。

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
