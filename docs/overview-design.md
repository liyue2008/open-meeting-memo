# OMM 概要设计（Rust 路线 B、CLI 与自动纪要流程已确认，进入首版实施）

- **已确定**：首版 macOS＋Lark/飞书＋Pi＋CLI；最终跨平台、多会议软件、MCP/WebUI 目标保留。
- **新增已确定**：用户目录遵循 XDG Base Directory Specification v0.8，macOS 同样遵循其默认值。
- **源码事实**：Yulu 在 macOS 13–14.3 用 ScreenCaptureKit，14.4＋用 Core Audio process tap；麦克风用 AVAudioEngine。
- **已确认**：Rust 主服务与 CLI＋照搬的 Swift 录音＋Rust 转写 worker 经 sherpa-onnx 原生 C 接口调用本地模型（路线 B）＋SQLite；首版不分发 Python 转写运行时，也不自动回退到路线 A。
- **已确认**：首版 CLI 功能清单；核心、录音与原生转写已接线，集成代码已通过受控测试及独立复审，真实会议验证另列。
- **首版目标基线**：macOS 13＋/Apple Silicon；录音 helper 按此编译，但当前官方1.13.2原生ASR包依赖的ONNX Runtime1.24.4最低为macOS15.5，较旧系统完整兼容仍需同版库构建及实测。Intel 待验证，不改变最终跨平台目标，不将未测试平台标为已兼容。
- **已确认**：逐字稿生成后直接调用已配置的 Pi 模型生成纪要，不再设置文本外发二次确认或授权阻塞步骤。
- **实施状态**：用户已要求继续首版实施；飞书检测配置及启停参数按下述建议实现并验证。真实系统授权、后台服务安装、模型下载及录音仍由显式运行操作触发，开发与测试不自动执行这些操作或调用付费模型。

## 一、范围、录音与检测事实

**PRD 确定事项**：自动感知会议启停、获取信息、录音、会后转写及纪要；后台运行；产物本地存储；无需管理员权限；录音实现务必照搬 Yulu。首版不做 MCP/WebUI，不能删除最终 Windows/Ubuntu、多会议软件与多模型接口目标。用户现已确定目录须遵循 XDG，此项不再列入批准清单。

**源码事实**：Yulu 输出源分离的 48 kHz/16-bit 双声道 WAV，左为麦克风、右为系统音频，保留其 PCM 处理、双轨同步、WAV 标记、路径防护、定期修补文件头与恢复逻辑。14.4＋使用全系统 process tap 和运行时私有 aggregate device，**不是安装虚拟声卡/驱动**；13–14.3 用显示器关联的 ScreenCaptureKit 流，仅接收音频输出。系统轨不限定飞书，其他应用声音也可能进入。

需要麦克风、屏幕/系统音频或系统音频录制授权；读取窗口标题还可能需要辅助功能授权。无需 root 不等于无需授权。签名升级、后台首次授权及不同 macOS 版本行为待实测。

Yulu 检测组合窗口标题、会议子进程及可选 `lark-cli`：查询活跃会议，确认当前用户 participant 状态为 `2`，再以新鲜 RTC 日志 room ID 交叉确认。日志只扫描少数新鲜文件的有限尾部，不是官方稳定事件 API；活跃会议列表不证明本机加入。强来源可得 ID/标题，窗口只有标题，进程多为泛化名称；不能承诺完整参会者、主持人或议程。Yulu 默认未开启此 CLI 检测路径或自动录制。

## 二、Rust 技术选型与 ASR 路线

**用户已选择 Rust 核心与原生转写路线 B。** 重计算原本已由 sherpa-onnx/ONNX 原生引擎执行，Python 是分段、批处理和协议胶水；换 Rust 不会自动让同一模型推理快很多。潜在收益主要是更低的常驻运行时开销、单独原生服务/CLI 分发、类型与资源所有权约束，以及减少对主服务 Node 的依赖；实际 CPU、RSS、转写实时因子和端到端耗时需要相同样本基准，不给无依据倍数。Pi 的模型耗时及 ASR 通常比命令解析、SQLite 更重要。

| 层 | 推荐 | 取舍 |
|---|---|---|
| 主服务/CLI | Rust stable；clap、tokio、serde/serde_json | 保持一个轻量应用 crate 加清楚模块边界，不先拆大型框架；原 TypeScript 方案依然可行，但常驻 Node/原生扩展打包更多 |
| 状态/诊断 | rusqlite＋SQLite、tracing；JSON 配置 | DB 串行访问/事务领取；不做分布式队列，敏感日志默认关闭 |
| 录音 | Yulu Swift helper 原实现，Cocoa/AVFoundation/Core Audio/ScreenCaptureKit | 不用 Rust 重写录音，不引入驱动 |
| 首版 ASR（已确认路线 B） | Rust 独立 ASR worker＋sherpa-onnx C API/FFI，FireRedASR INT8＋Silero VAD，CPU | 通过原生 C 接口调用现有推理库，不分发 Python；原生接口覆盖所需模型及有声片段检测，但 C++/ONNX 库、权重和 Rust 包装仍须打包与验证。Rust 同时迁移 WAV 读取、源分离重采样、去重与产物提交，验证与 Yulu 参考结果等价 |
| 纪要 | 外部 Pi CLI，一次任务一个子进程 | 用户安装/授权的 Pi 仍需其 Node 运行时；OMM 改 Rust 不消除 Pi 的 Node 依赖 |
| 分发 | 用户目录内签名/公证 App bundle，带 Rust CLI/service、Swift helper、Rust 转写 worker 及 sherpa-onnx/ONNX 原生库；模型显式下载 | 稳定独立 TCC 身份，依赖固定版本及哈希；不冒用 Yulu 签名/授权 |

路线 B 对固定版本 C 头文件作最窄 safe wrapper，约束指针生命周期、创建/销毁、线程与错误处理，并用独立 worker 隔离原生崩溃。以 Yulu 的现有实现为一致性参考，保留重采样、VAD padding、批量解码、时间戳和双源去重契约。**原生接口存在是源码事实，OMM 的 FFI/模型/打包组合未实测；不把 Rust 第三方绑定宣称为成熟现成方案。** 实施中先验证同样本结果与原生库打包，再建立性能基线；路线 A 不进入首版实现、分发或自动回退。若路线 B 验证失败，报告具体阻碍，不擅自改回 Python。

最低 macOS 13、首发 arm64 仍为建议：依据是 Yulu 录音 helper 的现有编译目标；选择路线 B 后，不再以原 Python runtime 的 cp313/arm64 包作为 OMM 转写架构限制。Rust 与原生推理库的目标架构组合仍须验证，不能据此承诺 Intel 已兼容或断言不能实现。FireRedASR 权重约 1.74 GB，不等于实际内存需求。首版中英、会后离线转写，不做实时字幕/翻译/说话人识别。

## 三、架构与“照抄”边界

```text
Rust CLI ── 私有 Unix socket ── Rust 用户级服务
                                ├─ MeetingAdapter(Lark)：观察/信息/置信度
                                ├─ 生命周期协调器 ── CaptureAdapter(macOS)
                                │                    └─ Yulu Swift helper → WAV
                                └─ SQLite 持久任务
                                      → ASRAdapter(Rust worker) → 转写提交
                                      → SummaryAdapter(Pi CLI) → 纪要提交
```

核心依赖契约，adapter 依赖核心类型，由入口静态注册，不扫描插件目录或执行任意插件。会议观察给 active/inactive/unknown、来源 ID、证据等级和时间；录音给真实路径/状态/错误；ASR 给时间片段与 mic/system 源标签；纪要给文本及模型运行信息。未来平台、会议软件、OpenAI 兼容接口及 MCP/WebUI 只替换边缘 adapter，不把首版扩为通用平台。

Swift 音频后端/PCM/WAV/恢复逻辑按参考版本直接移植，只改独立产品身份、打包和最窄外围连接。沿用 JSON socket 的 start/stop/status/windows/audio_devices，请求写完关闭写端再读响应。需要 App bundle、用途声明、audio-input entitlement、签名及用户级启动管理，不只是裸二进制。

现有 `meeting_daemon.py auto_stop` 静音回调需在最窄外围调整启动位置并桥接到 Rust 服务，首版不分发兼容 Python 桥接，**不改音频采集/写入内核**。无桥接时 helper 可能静音后自行停止，必须验收。原 helper 从状态恢复录制，服务重启应先查询并对账，不能重复 start。

## 四、XDG 目录（已确定）及服务一致性

下表均为最终用户目录设计，不是当前已创建的文件。所有 app 子目录用小写 `omm`。默认值在 macOS 也相同；撤销上版 Movies、Application Support/Caches/Logs 的 OMM 数据目录建议。

| 分类 | 变量下的相对路径 | 未设/空时默认位置 |
|---|---|---|
| 配置及录音策略 | `$XDG_CONFIG_HOME/omm/config.json` | `~/.config/omm/config.json` |
| 原始音频/元数据 | `$XDG_DATA_HOME/omm/meetings/<日期>/<UUID>/` | `~/.local/share/omm/meetings/…`，原 WAV 段、meeting.json |
| 转写/纪要 | 同会议 DATA 目录 | transcript.json/md、summary.md；保留版本及 hash |
| 本地 ASR 权重 | `$XDG_DATA_HOME/omm/models/` | `~/.local/share/omm/models/` |
| 任务 DB/重启状态/领取及恢复记录 | `$XDG_STATE_HOME/omm/` | `~/.local/state/omm/state.sqlite`、capture/ 等 |
| 日志/诊断历史 | `$XDG_STATE_HOME/omm/logs/` | `~/.local/state/omm/logs/`；默认不含会议全文 |
| 可重建重采样/解码临时文件、下载临时文件 | `$XDG_CACHE_HOME/omm/` | `~/.cache/omm/`；启动清理过期任务残留 |
| 产物提交临时文件 | DATA 内对应会议目录的私有 staging | 与最终文件同文件系统，以便原子替换；提交后清理 |
| socket、PID、进程锁 | `$XDG_RUNTIME_DIR/omm/` | **标准没有默认路径**，见下文 |
| CLI 可执行入口 | `$HOME/.local/bin/omm` | `~/.local/bin/omm`，显式安装；此处没有 XDG_BIN_HOME 标准变量 |

模型是可删除重下载的文件，但为断网可转写、避免系统清缓存导致失效，推荐把**已显式安装的模型**作为 DATA 保留；只把下载中间文件放 CACHE。模型移除不删会议产物，移除后离线转写进入 blocked，不能自动联网补齐。Pi 专用配置/凭据目录放 CONFIG 的 `omm/pi/`，不复制用户全局 agent 目录；短期工作空间放 CACHE，session 默认不落盘。

四个持久 XDG 环境路径须为绝对路径，相对值忽略并诊断，不将字符串 `~` 当环境变量中的绝对路径；RUNTIME 相对路径或路径攻击则拒绝，不悄悄回退。DATA/CONFIG/STATE/CACHE 未设或为空采用表中标准默认；按 XDG 优先级读取用户配置及系统默认，用户配置优先，不对这些搜索根写入系统文件，也不从数据目录自动加载可执行资源。新建私有目录0700、文件0600；已有 XDG 根不擅改权限，过宽的 OMM 私有目录报警或拒绝敏感写入。

**标准 RUNTIME 要求与本产品修订**：标准 XDG_RUNTIME_DIR 属于本用户、0700、本地文件系统支持 Unix socket/锁；其外部提供者负责从首次登录到最后退出共享并清空的生命周期。规范没有 /tmp、/run/user 或 ~/.cache 的统一默认。OMM 校验显式目录的绝对 canonical 路径、UID、0700、本地文件系统和 socket 长度，不声称能证明外部提供者的登录管理；运行目录不存持久任务、音频、模型或大型临时文件。

**用户已明确接受 macOS 替代目录＋最佳努力清理，修订了此前“完整登录周期否则阻塞”的过强承诺。** 有效显式 XDG_RUNTIME_DIR 优先；未设、空值或确认为不存在时，平台 adapter 通过 `confstr(_CS_DARWIN_USER_TEMP_DIR)` 获取系统用户临时根，安全创建/校验其私有 `omm` 目录，并向 stderr 和 JSON warnings 输出一致的降级说明。忽略 TMPDIR，不从 STATE/CACHE 或任意 locator 定位 socket。路径穿越、相对路径、symlink、用户不符、宽权限、非本地文件系统或无法安全检查均拒绝，不掩盖攻击后回退。

CLI/service/helper 使用同一根；连接验证 UID、服务实例、进程存活、0600 socket 与实际持有的进程锁。运行根和持久 STATE 各有一把锁，同一运行根或同一 STATE 仅允许一个服务，因此同 UID 多登录不能并发重复接管同一会议；只有显式隔离两根才允许独立实例。正常退出清理自身 socket/实例记录及自身 helper，保留稳定锁 inode，避免新旧等待者分裂锁。强杀后重启仅清理经 UID/STATE/实例记录和进程锁确认失效的资源，不删除活跃实例或身份未知资源。替代目录不是标准 XDG_RUNTIME_DIR，不保证复杂多登录最后退出或 SIGKILL 后立即清空，也不冒称完整登录管理。长 AF_UNIX 路径明确诊断，不静默改用持久目录。

**LaunchAgent 一致性**：不能假定它继承 shell 环境。显式 `service install` 时将解析后的四个持久 XDG 根及 CONFIG 定位写进 LaunchAgent 的绝对路径启动参数/EnvironmentVariables；CLI 在安装模式读取这个已固定的服务 profile，遇到当前 shell 根不同则诊断冲突，不启动第二套数据实例。未安装模式按当前 XDG 环境/default 解析。修改持久根须显式重配服务，不由 status 自动搬数据。安装时有效显式 XDG_RUNTIME_DIR 根也写入受校验的 profile 与 LaunchAgent 环境；CLI 根冲突时失败，根确实消失时允许双方带诊断退化。macOS 替代根不固定进跨登录 plist，每次由系统重新解析，不信任未验证 locator。

`~/Library/LaunchAgents/<OMM label>.plist` 是 macOS 要求的**启动入口例外**，可选可见 App 安装位置也是系统集成，不把会议数据放 Library；CLI 入口可链接 bundle 内固定二进制，不自动改 shell rc/PATH。用户 App 分发位置与安装行为须显式指定。后续 Windows 按平台实现权限/IPC，但数据分类与 XDG 要求保留，不默默改目标。

**helper 映射**：把 `YULU_APPLICATION_SUPPORT_DIR` 指向 STATE/omm/capture，`YULU_LOG_DIR` 指向 STATE/omm/logs，`YULU_MEDIA_LIBRARY_DIR` 指向 DATA/omm/meetings，`YULU_IPC_DIR` 指向当前 RUNTIME/omm 的 capture 子目录。配置由 Rust 读取 CONFIG，只在必要时生成最小兼容投影至 STATE；旧配置读路径须隔离，不能回落读取真实 Yulu 用户配置。`YULU_SCRIPT_DIR` 指向受信任的 bundle 桥接位置。上述已有环境参数及最窄外围映射替代 helper 内置 Movies/Library 默认，录音内核不重写。

## 五、生命周期、产物和持久任务

候选→稳定确认→录制→结束待确认→已封存；检测失败为 unknown，不是 ended。首次启用自动录音需知情同意；强信号稳定15秒、消失宽限30秒后重查停止仅为建议。窗口/进程弱信号只提示候选，可手动开始。**轮询及确认延迟可能漏录会议开头或多录尾段，15/30秒不是已批准指标；PRD 未定义允许延迟，应与用户确认并实测。** 不通过未经同意的常驻预录来补开头。无准确来源 ID 时生成 UUID，保留字段来源与 unknown，不编造会议信息。

同会议短暂重连保留 session，helper 已停则追加 segment 并记录真实缺口；超过宽限后关闭，再加入形成关联的新 session。不同强来源 ID 按现有稳定确认时间形成新候选，先成功封存旧会议再建立独立 session；短抖动和 unknown 不切换，手动录制不因自动观察改身份。睡眠前尽力封存、唤醒重查会议与 capture 状态；设备切换沿用原生重建，失败告警/留缺口，不能把静音当成功。手动停止抑制本会议自动重启；首版只允许一个录制 session。元数据记录时间/时区、会议来源、段/缺口、采样格式、hash、ASR/模型/提示版本、批准策略及处理状态。标题不是主键。

SQLite 为任务真源：封存→queued→transcribing→transcript_committed→summarizing→completed；另有 blocked/failed/interrupted。单 worker、事务领取租约、有限退避；按 session、输入hash、阶段与配置版本幂等。重启回收过期领取并恢复对账；录音未封存不交 ASR。临时产物校验后原子提交，DB/文件之间用提交记录恢复。纪要失败只重试纪要；显式重新处理创建 attempt 保留旧产物，不能因重试删除原音频。缺模型/Pi认证、损坏或磁盘满需具体 blocked/failed，不无限重试。

Rust 源码建议 `src/core`、`src/adapters`、`src/service`、`src/cli`，辅以 `native/yulu-capture`、`workers/local-asr`、`packaging`；不因换语言搭通用框架。

## 六、首版 CLI（功能清单已确认，已接线并通过受控复审）

用户已确认首版 CLI 功能清单；以下为对应接口的概要设计，尖括号是参数占位。确认功能不代表实际安装、录音或下载已执行；纪要生成直接接续转写，不另设文本外发确认。示例是产品接口说明，不是本轮运行真实录音或付费模型的指令。

| 分组 | 功能与示例 |
|---|---|
| init | `omm init --check`：只读检查目录/依赖/策略是否齐备；`omm init`：显式创建配置，不自动下载、安装服务或发文本。`omm permissions check` 只报可查状态，未知不能当已授权；`omm permissions request` 显式引导系统授权，可能需要桌面交互 |
| service | `omm service install`、`start`、`stop`、`status`、`uninstall`：用户级注册/生命周期。install 显式固定持久根；uninstall **只注销服务，不删除会议数据/模型/配置**；录制中 stop/uninstall 先封存并报告，不能静默丢弃 |
| record | `omm record auto enable --confirm-recording-consent` / `disable`；`omm record start --title "讨论会"`、`stop`、`status`。auto 是持续检测策略，不是手动起录；开启须批准知情策略，弱信号规则不变 |
| meeting | `omm meeting list --from 2026-10-01 --query "项目"`：日期/来源/状态/关键词检索；`omm meeting show <id>`：元数据/产物索引；`omm meeting read <id> --artifact transcript`（或 summary）：明确读取文本，不在普通 list/status 中倾倒全文 |
| process | 统一入口 `omm process <meeting-id> --stage transcribe|summary|all`；如 `omm process <id> --stage transcribe --wait --timeout 20m`。只处理已封存段；summary 依赖已提交转写。`--retry` 重试失败/blocked阶段，`--force` 创建新attempt；不另建 transcribe/summarize/reprocess 三套入口 |
| task | `omm task list --status failed`、`show <task-id>`、`wait <task-id> --timeout 10m`；按 task 触发重试用统一 `omm process --task <task-id> --retry`，与按会议参数互斥，不再另设 task retry |
| model | `omm model list` / `verify <model-id>` 只读状态/哈希；`omm model download <model-id>` 显式联网并校验，完成后原子安装。支持单模型 `remove <model-id> --confirm`，须无在用任务；不做会议批量删除 |
| config/paths/doctor | `omm config show`（脱敏）、`validate`、`set <key> <value>`；`omm paths --json` 显示解析根/来源/安装profile冲突；`omm doctor --json` 只读诊断权限可见状态、模型/接口/服务就绪，不能自动修复、触发录音或联网下载 |

**AI Agent 契约建议**：所有命令支持 `--json` 稳定、版本化结果（schemaVersion、ok、data/error、warnings），stdout 不混进日志，诊断到 stderr；全局 `--non-interactive` 禁止追问，不替代授权。录音需要确认而无显式同意时返回 consent_required，不能把通用 `--yes` 当录音同意。纪要生成不要求文本外发确认参数。权限请求在无桌面环境返回 interaction_required，不声称可绕过 TCC。

长任务默认返回 task ID；`--wait --timeout` 等待结果，等待超时不取消已持久任务。拟定退出状态：0 请求成功（入队不代表处理完成，必须看 task 状态）；2 参数/配置无效；3 缺依赖、授权或必要前置；4 执行失败；5 等待超时；6 服务不可达/状态冲突。结果保留结构化错误码区分详细原因；`--wait` 的成功须对应目标阶段完成，blocked不是成功。手动 process 与自动会后管线共用同一队列/幂等检查。

下载、服务安装和移除都由显式操作触发，不由 help/status/init --check/doctor 副作用触发。逐字稿提交后自动进入 summary 阶段，直接调用已配置的 Pi 模型；不提供 `--allow-text-upload` 开关，不增加文本外发授权检查或人工批准步骤。显式 summary/all 命令也使用同一流程，不静默换模型。普通错误不含会议文本、密钥、OAuth token；内容仅在 read 或明确内容检索中返回，config show 永不输出凭据。自动处理配置快照锁定在任务创建时。

**后续而非首版**：MCP/HTTP WebUI、实时字幕/翻译、说话人辨识、跨平台录音 adapter、多会议软件及多 Agent/模型接口、会议批量删除与复杂分享功能。已有单模型移除不扩成全数据清理命令。

## 七、Pi、安全与后台边界

沿用已安装官方文档确认的 `pi --print` 单次输出模式，error/aborted 非零退出。Rust 用固定绝对 executable、argv、选定 provider/model 启动外部 Pi，文本由 stdin 输入；禁工具/MCP/extensions/skills/templates/context discovery/session落盘，拒项目trust。独立工作目录、CONFIG 下专用 `PI_CODING_AGENT_DIR`、最小环境、用户为此专用 Pi 授权，不继承整个 agent 目录。超时/输出上限及结果校验后由 OMM 提交产物；Pi 不需要音频或DB。长会议分时间块提炼再汇总、带来源时间，预算超限 blocked。

这是能力收缩，不是 OS 沙箱；工作目录不能限制同用户进程访问。会议文本为不可信材料，内容中的指令不授权工具或配置更改。音频默认不上传，ASR在本地；**云端 Pi 模型可能收到转写和必要元数据，本地落盘不等于全程离线**。逐字稿生成后直接调用已配置模型总结，无需额外确认；用户可配置云端或本地模型端点。模型调用是否离线取决于所配置的端点，不承诺云模型调用全程离线。默认无分享、遥测或全文日志。

用户级 LaunchAgent、不用系统 daemon/sudo；socket0600、runtime目录0700、敏感文件0600，校验 realpath/产物根、拒路径穿越。同用户恶意软件不在文件权限可隔离的威胁边界内。网络凭据不写普通 config，诊断不得打印。

## 八、风险、实施计划与批准项

**仍未实测**：飞书日志格式/CLI版本/API权限会变；后台/升级TCC授权可能变化；全系统其他声音、飞书静音时本地麦克风可能被录入（原mute classifier不含Lark）；蓝牙/采样率可能漂移，原代码按48k写头需实测；原 RIFF UInt32 约6.2小时上限，建议外围按小时 stop/start 分段并记录间隙，不擅改内核；VAD/跨源去重可能漏字或误删，双轨不是参会者diarization。新增风险是 macOS runtime fallback 的最佳努力清理边界/安全定位、XDG服务根一致性，以及 Rust重采样/FFI与旧结果等价性；尚无“换Rust就更快”的基准。

**首版实施的五阶段**：①Rust最小骨架、XDG解析/服务profile/runtime机制，以及签名helper协议/权限；②Lark强弱信号与生命周期；③SQLite恢复、本地产物与已确认的Rust原生ASR路线B；④隔离Pi与完整CLI；⑤用户级服务/分发与长会议故障验收。验证持久XDG相对值忽略而runtime路径攻击拒绝、持久根一致、runtime来源/warning/单服务锁/正常清理/强杀恢复、删缓存不丢音频/模型；录到双方、不重复启停、弱信号不误录、拒权可诊断、断网可转写、kill/restart保封存音频、重试不重复产物、睡眠/重连记录缺口、音频不上传、转写后直接生成纪要且不要求额外确认；授权中英样本建立准确率/耗时/RSS基线，并量化漏录开头/尾段。

**已确认**：XDG目录要求、Rust核心与原生转写路线B、首版CLI功能清单；按PRD直接移植Swift录音内核，不改用其他采集方案。

**实施授权与运行边界**：用户在确认路线B与CLI后，进一步要求删除文本外发确认并继续；按当前设计进入首版实施，不重复请求已确定的路线或纪要授权。首版先在macOS13＋/arm64目标验证，可选已授权lark-cli、弱信号不自动录音、候选15/30秒及分段策略作为可配置建议实现并实测，不把未实测指标写成保证。实际系统授权、服务安装、模型下载和录音仍由显式运行操作触发；开发与测试使用合成或专门授权样本、假模型子进程，不使用个人会议材料或付费调用。若无飞书API也要精准自动启停，目前证据不足，记录兼容性限制，不虚构能力。

## 证据附录

沿用首次只读调查版本：Yulu `66f8468634b27933c23865da1e7c09b50bd3cf5a`；当时OMM `672cbac9fa60193e131e6c5614fa0b7c0f8b9796`。本次读取当前设计文档，未重新声称当前OMM HEAD相同。以下为源码/官方文档证据，不是OMM实测结果。

- [prd.md：功能/特性](../prd.md#功能特性)、[首版设计要求](../prd.md#设计要求)：最终与首版边界。
- `Yulu reference tree/yulu/scripts/audio_daemon.swift:398–500`：RIFF UInt32及5秒修补头；`Yulu reference tree/yulu/scripts/audio_daemon.swift:850–921`：mic/system源分离；`Yulu reference tree/yulu/scripts/audio_daemon.swift:1186–1240`：设备格式/route-change重启；`Yulu reference tree/yulu/scripts/audio_daemon.swift:1361–1475,1560–1738,2401–2420`：两后端与版本选择。
- `Yulu reference tree/yulu/scripts/audio_daemon.swift:225–250,611–638,2383–2398`：静音回调/识别范围；`Yulu reference tree/yulu/scripts/audio_daemon.swift:2136,2170–2200,2243–2343,2453–2464`：socket权限/framing/动作/恢复；`Yulu reference tree/yulu/scripts/audio_daemon.swift:88–110,193–207`：已有目录/录音root环境覆盖能力。
- `Yulu reference tree/yulu/scripts/build_audio_daemon.sh:47,212–226`：arm64/macOS13及用途声明；`Yulu reference tree/yulu/scripts/Yulu.app.entitlements:5–6`：audio-input。
- `Yulu reference tree/yulu/scripts/meeting_detector.py:47–55,367–496,562–609,803–866`：日志、参与状态交叉验证、兜底及结束逻辑。
- `Yulu reference tree/yulu/scripts/sherpa_offline_worker.py:71–106,144–180,230–269`：CPU FireRedASR、VAD及源级结果；`Yulu reference tree/yulu/scripts/yulu_ui/src/offlineTranscription.ts:77–103,122–152,256–280`：中英限制/双轨预处理/去重；`Yulu reference tree/yulu/scripts/pcmResampler.ts:5–74`：16k抗混叠重采样，换Rust核心仍需迁移验证。
- `Yulu reference tree/yulu/scripts/local_caption_runtime.py:78–106`：模型大小及hash；`Yulu reference tree/yulu/scripts/local_caption_runtime_pack.json:2–6`：1.13.2/cp313/arm64。OMM runtime manifest/签名应自有，不原样要求Yulu签名主体。
- `Yulu reference tree/LICENSE:1–21`：MIT复制需保留版权、许可、免责声明；模型、依赖、图标和打包runtime许可证仍须逐项核对，不宣称均可再分发。
- **新增官方规范**：[XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/latest/)（完整读取，v0.8，§2–4）：四个默认根、绝对路径/相对值忽略、STATE日志/恢复、`~/.local/bin`、runtime所有权/0700/登录生命周期/本地文件系统、未设replacement＋warning及不放大文件、创建/查找/优先级条款。
- **新增原生ASR证据**：[sherpa-onnx v1.13.2 C API](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.2/sherpa-onnx/c-api/c-api.h#L892-L898) 为 `SherpaOnnxOfflineFireRedAsrModelConfig`；同文件 `#L1078-L1079` 挂接离线model config，`#L1246-L1248` 创建recognizer，`#L1291-L1292` 创建流，`#L1351-L1353` 输入音频，`#L1412-L1433` 单/多流解码，`#L1524-L1538` 结果与销毁；`#L1847-L1866` Silero配置，`#L1920-L1933` VAD配置，`#L2082-L2084` 创建VAD，`#L2109-L2119` 输入/队列为空接口。证明C接口覆盖所需模型/VAD，不证明OMM Rust绑定与打包已完成。
- Pi官方包参考根 `installed Pi package @earendil-works/pi-coding-agent/`：`package.json:2–3` 为1.1.0；`README.md` Getting started为Node22.19＋；`docs/cli.md:43–49,98–99,125–126,196–215,241–242` 为单次及能力收缩；完整读取过 `docs/cli-integration.md`、`docs/configuration.md`、`docs/environment-variables.md`、`docs/security.md`、`docs/containerization.md`，沿用其退出状态、agentDir及无内置沙箱限制，不读取任何个人配置/凭据。

**路线 B、CLI 功能清单及转写后直接生成纪要已确认；用户要求继续，已进入首版实施阶段。实际兼容性与测试结果由实施证据确认。**

## 当前实现状态（既有基线受控复审通过，新增恢复修复待复审）

- capture/asr 原生组件已接入单 crate 默认构建；录音遵循 EOF JSON socket，转写使用独立 Rust worker 与 sherpa-onnx 1.13.2 C API。未修改 Swift 内核或 native 转写算法的所有权边界。
- 核心服务/CLI 已接线：强弱信号、知情同意、录音封存、提交 journal、自动纪要排队、租约续期、有限退避、重启对账与缓存清理。转写后直接调用专用 Pi print 配置，输入只含文本/时间，不传原始音频；无单独文本外发审批。
- `summary --force` 复用校验过的提交转写，保留原 ASR 来源/hash/配置，创建新的纪要 attempt；不自动重新转写。卸载注销服务，不删除用户会议数据；封存不明确时拒绝卸载。
- **macOS runtime replacement 已按用户修订接入**：安全系统用户临时根、稳定 rendezvous、UID/0700/本地路径/socket/进程锁/实例校验、warning、正常清理和强杀后失效资源恢复；明确最佳努力而非完整登录周期。未设显式 runtime 的服务/多 CLI、重复服务与同 STATE 分裂保护、恶意权限/symlink、正常清理及 SIGKILL 重启使用隔离假服务回归验证。四持久根与安装 profile 冲突保护不变；显式 runtime 同样冻结/校验，不把替代根当固定 locator。
- 飞书观察在独立、有界的单任务中运行：总预算 8 秒，最多 8 次参与状态查询/8 个日志目录；失败为 unknown，取消后等待观察子进程回收，不与下一轮未收束任务叠加。控制请求/封存/轮换不等待飞书查询，配置或手动控制变更使旧观察结果失效。
- Pi workspace 以实例、任务/session/attempt、STATE 和 UID 标识并持有锁；每次 spawn 前持久记录 child 未知，spawn 后记录实际 PID。取消时先 kill+wait 所持 child、再释放目录；服务退出等待回收。恢复只清理锁空闲、所有权/任务一致、父与子均确认死亡的目录。SIGKILL 父后孤儿可能继续模型调用；活跃、PID 复用、权限错误或记录/路径未知均保留诊断，不凭 PID 擅杀、不删除另一 STATE 的工作空间。
- 已接管 helper 的正常退出已接线：封存成功后，无论是否持有 child 句柄都调用实例绑定的 shutdown；验证实际进程退出及端点消失，持有 child 时再 wait。正常 stop/uninstall、错误身份/录制中/quit 失败拒绝、缺失端点与从未启动区分、封存失败保留音频均有 core 回归；真实 Swift `--ipc-fixture` 合成回归覆盖预启动接管后 stop/uninstall 及服务启动 child 后退出。卸载不把活服务的身份拒绝误当服务缺席，不强制 bootout；遗留 helper 未对账时保留安装状态。仅音频外围退出协议变更，音频内核不变；已通过独立复审，不宣称真实录音或发布验收通过。
- 假 helper/Pi/worker、合成音频和 C API 夹具验证，不替代真实系统授权/录音、官方库/权重推理、整小时准确率/性能、睡眠/设备切换、macOS 13 实机或发布验证。诊断中的 unknown 权限和准备状态不等于已获系统授权；安全收缩不是操作系统沙箱。

### 用户级安装验收补充

已按[安装指南](install.md)在用户目录验证 CLI、开发签名 bundle、真实用户 LaunchAgent 注册/启动/正常停止/卸载保留数据，auto 保持关闭。官方1.13.2动态库及已校验权重成功处理自产中英文短 WAV，Pi 成功对仅本次文本生成短纪要。它们不替代真实硬件采集、系统权限或正在进行的个人会议检测；无人桌面授权下 capture IPC 超时，具体权限仍 unknown。

启动错误恢复新增显式 `Recording.start_confirmed`：新意图未确认，匹配开始响应或可信 session 对账后确认，legacy 缺字段默认已确认以保守保护。仅未确认、helper 明确 idle 且无 session/path/error、私有 canonical 会议目录全空且无音频段/封存/index 时可清理意图；失败启动的 gap/rotation 标记不意味着已有音频。不清理有任何文件、不同/未知 helper、已确认或 legacy 记录，不删除音频。helper 启动重试有15秒总预算；尚未授予权限不能变成就绪承诺。参考 Yulu 的路径/行号均指移植参考树，不是贡献者必须拥有的个人目录；移植版本及 hash 见[provenance notice](../native/yulu-capture/NOTICE.md)。

### 桌面通知边界

核心使用 `NotificationEvent` 类型化状态，独立 `NotificationAdapter` 只接收枚举，不接收会议标题/正文/凭据/原始错误。Controller 在匹配的真实 start 响应（或未确认意图的可信 recording 对账）及成功封存/任务入队后发事件；轮换与重连保留一次会话语义。pipeline 在失败状态持久化后发阶段错误，在 summary 提交成功后发完成，转写单阶段不会发纪要完成。

macOS adapter 调用系统 osascript 的固定 AppleScript，消息来自编译期常量 argv；最小环境、不加载个人脚本，不改录音内核。没有新增独立 App 通知身份/权限；执行宿主身份与桌面显示由 macOS 管理。16槽非阻塞队列、3秒调用预算、owned-child 取消回收；通知失败/拒绝不影响任务结果。只有会话/任务attempt成功事件持久去重；所有失败事件按阶段跨任务共享60秒冷却，不使用永久 task receipt。时钟可注入，生产使用持久 UTC 毫秒：重启保留冷却，跳过的失败不延长冷却，同一 attempt 后续重试在冷却结束后仍可提醒；时间回拨重新建立冷却。调用拒绝/超时也消耗冷却以防刷屏，不修改业务 attempt 或产物。成功 receipt 表示调用尝试，不代表系统递送，崩溃仍可能漏通知。正常退出队列有3秒处理预算后取消；子进程回收超过监控预算时记录等待提示，继续等待 owned exit，不承诺总退出≤3秒，不丢弃活进程所有权。

`notifications.enabled` 默认 true，`notifications.events` 允许筛选泛化状态，旧配置兼容。显式 `notifications check/request/test` 分别只读检查机制、打开系统设置、发送无敏感测试；权限与可见性 unknown，用户在系统界面操作，不能脚本授权。未增加 MCP/WebUI、付费模型调用或录音授权。实际 daemon 集成夹具在临时 XDG 根注入 fake notifier，运行同一 select/自动观察/捕获 IPC/自动管线/退出流程，用受控 helper/worker/Pi 验证成功时序、失败和子进程回收，不触及个人会议。真实后台可见递送、勿扰与拒权行为须桌面实测，不能以子进程 exit0 代替；安全升级保留已有 auto/consent 配置，活跃处理期间不替换服务。测试只能针对固定自产通知卡片取证，不抓取个人通知。macOS 13 全链兼容仍待实机验收。
