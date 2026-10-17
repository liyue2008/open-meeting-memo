# Desktop v1：冻结的前后端接口

状态：Foundation 交接基线；定义完整产品接口，**尚未实现业务命令**。原生命令实现归 native，界面实现归 frontend。任何接口变更先由 Main 安排一个共享契约 owner，再重新生成及验证，两个组件不得各自改口径。

## 文件与生成

- [`schema.json`](schema.json)：唯一结构定义，JSON Schema 2020-12；`x-commands` 定义 33 个命令的请求/响应/持久 mutation 分类，`x-budgets` 定义预算。
- [`examples/`](examples/)：33 个成功交换、2 个在线/离线授权交换、10 个已有核心错误码的**合成测试材料**，不是生产默认数据或实现证明。
- [`service-capability.examples.json`](service-capability.examples.json)：现有身份绑定 Unix socket 上的新增能力握手。
- [`commands.generated.rs`](commands.generated.rs)：Tauri AppManifest 命令清单；仅生成单项 ACL（访问控制）权限，不授予调用权。
- [`src/application/desktop_contracts.rs`](../../../src/application/desktop_contracts.rs) 与 [`contracts.generated.ts`](../../../apps/desktop/src/contracts.generated.ts)：生成的 Rust/TypeScript 数据类型。

```sh
node scripts/generate-desktop-contracts.mjs          # 明确修改 schema 后运行
node scripts/generate-desktop-contracts.mjs --check  # 不写文件，检测漂移
(cd apps/desktop && npm run contracts:check)
cargo test --locked --test desktop_contracts
```

生成器只处理当前实际使用的 schema 词汇；未知类型直接失败。Rust 引用现有 `core::contracts::ErrorCode`，不另设不一致错误码。Rust serde 检查类型、未知字段、枚举、必填 nullable；**数值/长度/format/跨字段预算由 native 在入口与用例层校验**，不能把成功反序列化当成安全验证。TS 运行时同样必须验证实际桥响应，不得 `as any` 或失败后返回空成功。

## 传输和失败

命名 Tauri 命令调用形式：`invoke(command, { request: { schemaVersion: 1, requestId, payload } })`。
结果为 `{schemaVersion:1,requestId,data}` 或 `{schemaVersion:1,requestId,error}`，只能有一个分支；请求/响应 ID 必须匹配。
测试交换额外含 `command`，它不是请求 payload 的字段。所有对象禁止未知字段；所有 wire 字段 camelCase。既有 service 外层保持 snake_case 兼容，嵌套 desktop DTO 使用 camelCase。

- ID 为 UUID；毫秒、序号、revision 等整数必须位于 JS 安全整数范围（绝对值不超过 9007199254740991）。revision 字符串为小写 64 位 SHA-256；metadata revision 为整数 CAS（写入前比较版本）。
- requestId 为 1–128 个 ASCII 字符 `[A-Za-z0-9_.:-]`。持久 mutation 按规范 profile＋命令＋requestId 持久去重，并绑定规范化请求体 hash；同 key 不同 body 返回 conflict。重复传输返回原 operation，不新建 attempt。一次新的用户 force 使用新的 key。
- error 使用已有 10 个 code，附 nullable 的有限 `reason`、retryable、安全 message、字段错误。未知错误、未知状态/协议不能被前端默认为成功。not_found 用 unavailable＋not_found；取消为真实 task 状态或 reason，不冒充 timeout。
- message≤1024 字符、字段错误≤64 项；不可回传真实子进程 stderr、凭据、会议正文或绝对私有路径。内容只在显式读/导出中返回。列表可返回标题，但诊断及其导出不含标题/查询/正文/认证。

## 预算与语义约束

结构上界见 schema；下面是还必须执行的语义规则：

| 项目 | 上界 / 行为 |
|---|---|
| mutation JSON | UTF-8 128KiB，解析前拒超限；标题 1–256 Unicode 字符，note **UTF-8≤16KiB**，tags≤32 个唯一值、各1–64字符；query≤512字符 |
| 读响应 | 单次序列化正文≤1MiB（含 JSON 开销）；artifact 使用 cursor 分块；summary 还受已有配置输出预算限制 |
| 列表 | 默认25、1–100项；批量1–100项，不得重复 meeting ID；段/产物/任务/通知均分页 |
| 原生阻塞请求 | 每个可信窗口最多4个，有限排队/timeout；过期读取消，不取消持久任务 |
| 轮询 | 活动队列2秒、被动页面10秒；隐藏/离线退避，不无界并发 |
| import | 单文件≤2GiB、≤8小时、1–2声道、8–96kHz；非零帧；WAV PCM int16/24/32 或 float32，Ogg只接受验证过的Opus |
| 媒体 | 全局≤8、每窗口≤2个并发请求，单响应缓冲≤8MiB；媒体预算独立于正文桥预算 |

标题/note/tags 不改原始 `meeting.json` 或 capture 事实；metadata overlay 用整数 expectedRevision。会议删除只是 tombstone，永不清理旧音频/版本/检查点；恢复也比较 revision。正在录音、未封存、活跃任务或所有权不明拒删。批量 metadata/trash/restore 事务全成或全拒；reprocess 批量逐项明确 operation/error，不隐藏部分失败。

`meeting_list` 的日期区间为 `[fromLocalDate,toLocalDateExclusive)`，同时提交 IANA timeZone 与精确 UTC 边界。native 校验对应关系及先后顺序。前端默认包含今天的最近7个自然日；今日、本周（周一起）、最近3/7天按日历而非减24小时计算，覆盖夏令时。无准确开始时间显示 estimated/unknown，不伪造精确时间。搜索限定 title 或已提交 title/transcript/summary 的 content 投影。search.complete/indexed/pending 明确索引未完成，total 未知为 null。query/filter/sort/visibility 绑定 opaque cursor；响应 revision 变化必须重置分页，不能跨快照重复/漏项。排序以开始/封存时间或时长，UUID 为稳定次序。

`MeetingStatus` 是真实核心状态的 UI 投影，不能作为另一份持久真源。保留 recording/unsealed/partial/unknown 等区分。task status 中 cancelled 是待 native 完整实现的契约，不允许直接把新枚举写给旧 daemon。

`artifact_versions` 展示真实 attempt、hash、输入来源和当前/历史；当前内容选最新**有效提交**而非最新失败 attempt。summary 只有精确引用当前 transcript 才算 current；重处理失败仍可读旧的一致结果，旧 summary 显示 historical。transcript chunk 用 `lines`，summary 用 inert Markdown `text`；format 不允许 HTML。partial 不等同空终稿。artifactRef 的 meeting/task/stage 必须再次交叉校验。copy 只复制该已验证完整产物，≤桥上限，超过返回 budget_exceeded；不接受任意 renderer 文本。export/batch_export 由 native save dialog 控制目的地，不接收任意路径；用户取消返回 outcome=cancelled，而不是 saved。

`processing_enqueue` 明确 all/transcribe/summary，summary 可选择已提交 sourceTranscriptTaskId；多会议批量指定 source 只允许确实属于对应会议的来源。一次活跃 generation，双击/重传不能重复入队。retry 保持已有初次＋最多3次自动重试；显式 requeue cancelled 创建新 generation。cancel 是持久意图，取消 owned child 后先 await 退出再清理，提交与取消事务串行；提交已赢则返回 completed。界面等待超时不取消任务。未知/存活 orphan 不凭 PID 盲杀。

import_select 原生选择一份文件，仅返回短效 opaque handle；取消返回 selection=null。import_commit 带明确本地时间、时区和 metadata，验证后私有复制/hash、安全转换、原子提升/journal。调用者原件不动。import_cancel 等待 owned 转换进程退出后清 staging。重复字节返回既有可用 meeting 或明确冲突。公开 mix 必须如实标 imported mix，不能复制为两个虚构 mic/system 参与者。

## 配置和兼容写门禁（Main 内部补充已批准）

`EditableConfig`/`ConfigPatch` 是现有 `AppConfig::set` 的完整非秘密白名单 camelCase 投影：模型版本和 resolved roots 只读，auto/consent 只经 `record_auto_set`。禁止认证字段。patch omitted 表示不修改，`piNodeExecutable:null` 表示清除；Rust 用显式 present-null 反序列化保留区别。config_validate 只校验，不写磁盘。config_save 在配置写者锁内重读**原始字节 hash**、比较 expectedRevision（不存在为 null）、一次验证整个 patch、更新 UUID、0600 原子替换＋父目录 fsync。CLI init/set/auto 等每个写者都必须使用相同锁和 CAS。结构不完整与依赖/权限 unknown 分开展示；`ConfigEffect` 如实说明新任务/新段/reload/restart 生效，不声称立即生效。缺配置可显示明确标识的默认表单，但不能用默认值悄悄覆盖缺失字段。

App 打开现有 profile 仅只读：不迁移 SQLite、不改配置、不接管 daemon。握手 command 为 `desktop_capabilities_v1`，放在既有 `Request`/`Reply` 身份绑定 socket 协议中；新增 optional `desktop_capability` 使用 `ServiceCapabilityRequest`。旧 daemon 拒绝/未知命令、超时、能力未知一律无写权。Reply.service_instance、嵌套 serviceGeneration 与真实身份/锁必须一致。service 传输 `writeAdmission:null`；可信 native 根据当前窗口/profile 生成短效授权，再向 UI 输出。所需 capability 逐用例执行，不只检查 UI 按钮。

`WriteAdmission` 是**可验证的当前写入所有者 generation**，不是只靠服务版本字符串：

1. 在线兼容 service：核验身份、protocol、generation、状态 schema/min-reader 和所需 capabilities；mutation 持有相同 runtime/STATE/config 等相关锁直到提交，避免检查后换 daemon。
2. 无 service：仅**显式初始化的新 profile、隔离 fixture profile、或已可验证由新版本接管的 profile**可以建立 offline generation。仅 socket 不存在、超时或锁刚好可拿都不能证明旧 reader 不存在。旧个人 profile 不因一次打开或保存配置隐式迁移/接管。
3. offline 可做安全配置验证与原子 CAS 保存，无需模型/Pi/Lark就绪；录音/排任务等要求 service 的动作仍返回真实 unavailable。`admissionMode`、可用 capability、按钮原因准确投影，不能把 offline 写权当成后台功能已运行。
4. admission 绑定 ownerGeneration、规范 profile 的 profileId hash、可信调用者/窗口、随机 token 和 expiresAtMs；模式为 service/offline。token 不进日志/持久前端存储。失效、锁丢失、跨 profile 重放、generation/schema 变化立即 conflict。无授权时 `admissionMode=none`、writeAdmission=null、writesAllowed=false。
5. legacy/旧 daemon/所有权不明统一拒写并解释 requires_switch/incompatible_service/incompatible_schema；显式安全切换不得偷偷启停个人服务或改 auto/consent。迁移和回滚限制须 native 文档化。测试覆盖 offline 双进程/旧daemon同时启动/跨profile重放、旧库只读无迁移、检查与提交竞态、旧配置字节不变。

## 信任窗口与媒体协议

native 每次命令验证 main window＋webview＋当前本地 URL；ACL 只按显式命令授予，不给 remote capability 或 shell/fs/opener/default 大范围权限。release CSP 禁止远程脚本/网络、HTML frame/object、任意 asset DATA 路径。开发 Vite 只监听127.0.0.1，devCsp 仅在开发生效。会议 Markdown 禁 raw HTML、脚本、远程图像/iframe、不安全协议；外链不授权通用原生命令。

`media_manifest` 只列**验证后的公开 mix**：真实序号/offset/duration/hash/availability。老WAV只有存在可信原 manifest/hash 血缘才可读。不暴露麦克风/系统内部分源。`media_open` 检查 expectedRevision，返回绑定 main webview、meeting、segment、hash/revision、有效期的随机 grant；URL 严格 `omm-media://localhost/<grant>`，不接受路径/query token。native 验证协议回调来源，不能只信请求 header。每请求在共享音频读者锁内安全 nofollow/component-open/fstat/hash，再读同一已验证 FD；锁覆盖 hash/读取，重验身份。trash/版本失效/窗口关闭/过期/media_close 撤销 grant。

协议仅 GET/HEAD，单 Range（start-end/open/suffix）；206 正确 Content-Range/Length/Accept-Ranges、416 拒多范围/非法/越界；ETag=hash、If-Range 不匹配返回有界错误要求重开，不能假造完整200。no-store/nosniff/CORS仅可信本地 origin。不带 Range 的大文件必须安全流式或显式要求 Range，经实际 WKWebView 证明，禁止截断字节却宣称完整200。播放仅用户点击，不自动播放；fixture 只测 metadata/静音解码/暂停态段内及跨段 seek，**严禁调用 play()**。实际 Tauri/WKWebView 来源/Range/driver 可行性是两个组件开工后的最早 rendezvous，不得拖到最终验收或换 Chromium。
