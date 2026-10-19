# Open Meeting Memo (OMM)

<img src="docs/assets/omm-logo.svg" width="96" height="96" alt="Open Meeting Memo（omm）Logo">

本地录音和转写，使用已配置的 Pi 生成会议纪要。现有 CLI 基线面向 macOS、Apple Silicon、飞书；录音 helper 编译目标是 macOS 13+，本指南固定的官方 ASR 库依赖 macOS 15.5+，较旧系统未验证完整链路。不包含实时字幕、说话人识别、浏览器版 WebUI 或 MCP 服务。

**Tauri 2 桌面 App 正在独立分支整合验收**，支持目标为 Apple Silicon、macOS 15.5+。真实 React 界面、Rust 共享用例、受限媒体协议和原生包装组件已合入；正式独立评审、完整界面验收与最终用户安装仍以验收记录为准。参见[桌面开发与统一入口](docs/desktop.md)、[组件边界](docs/desktop-lanes.md)、[能力与CLI映射](docs/desktop-capability-matrix.md)及[验收状态](docs/desktop-acceptance.md)。已确认的[静态设计](docs/webui-demo/index.html)仅作视觉依据，不是生产数据源。首轮评审后的冷首次启动、部分逐字稿、导入清理与队列互斥修复已进入当前源组合；合成夹具通过不是新正式评审、用户安装或真实权限/识别质量的完成证明。普通App启动保留持久偏好；显式非持久生产启动检查及其隔离边界见[安装指南](docs/install.md#桌面-app-状态)，不以测试宿主冒充安装生产App。

桌面界面使用 Mantine 9.7.3 官方默认主题，支持明暗和侧栏收缩；播放器只显示一套按钮与会议时间滑条，同一个隐藏 audio 元素负责媒体。旧资料库恢复保持[显式采用、另行启动](docs/desktop-service-recovery.md)两步，未知录音状态不显示为未录音。组件与合成测试不能证明用户回声已经消失。

**当前为开发版本，不是已发布或验证过真实会议的产品。** 新分段版本待独立复审及用户级部署。录音按音频帧封存为最长30分钟的片段；每段封存后立即进入独立、可恢复的本地转写队列，会议结束后聚合转写并生成一次纪要。最终音频是48kHz、单声道的真实 Ogg/Opus（`.opus`），需要显式安装固定编码工具。受控测试及自产20/60秒语音基准没有验证真实录音、系统授权或长会议质量。

- [用户级安装、依赖、卸载与故障处理](docs/install.md)
- [贡献者/Agent 指南](AGENTS.md) · [概要设计及边界](docs/overview-design.md) · [需求](prd.md)

用户已接受 macOS 安全替代运行目录与最佳努力清理：没有可用的显式 `XDG_RUNTIME_DIR` 时使用系统用户临时区域，不把 STATE/CACHE 冒充运行目录，也不承诺完整登录生命周期管理。服务注册成功不表示后台或权限已就绪；不要把单测或开发构建当作实际授权、录音、真实模型推理或发布证明。

## 构建

需要已有的 Rust、Xcode/Swift 工具链，不由构建脚本安装全局工具。

```sh
cargo build --locked
bash packaging/capture/build-dev.sh
# 显式下载校验固定源码并在 target 构建，不安装系统软件或用户服务
bash packaging/audio/build-dev.sh
```

Cargo 默认启用 `capture`、`asr`；输出 `target/debug/omm` 与 `target/debug/omm-asr-worker`。开发录音程序位于 `target/capture/OMMCapture.app/Contents/MacOS/omm-capture`，开发签名不是公证或发布签名。Rust worker 使用 sherpa-onnx **1.13.2 C API**，推理由 C++/ONNX 执行，不能称为纯 Rust 推理。

## 目录与配置

`omm paths --json` 显示实际目录。持久目录均按 XDG 规范解析，相对环境变量值忽略并告警：

|用途|默认位置|
|---|---|
|配置与专用 Pi 目录|`~/.config/omm/`|
|音频、会议资料、转写、纪要、模型|`~/.local/share/omm/`|
|SQLite 任务、租约、恢复状态|`~/.local/state/omm/`|
|可清理的 worker/Pi 临时产物|`~/.cache/omm/`|

目录要求当前用户所有、0700；新建敏感文件为 0600。服务安装记录四个持久根，CLI 发现根不一致时失败，不静默读取另一套会议数据库。

运行目录优先使用有效显式 `XDG_RUNTIME_DIR/omm`；未设、空值或目录确实不存在时，在 macOS `confstr(_CS_DARWIN_USER_TEMP_DIR)` 提供的用户临时根中使用私有 `omm` 目录，不信任 `TMPDIR` 或自定义 locator。相对路径、路径穿越、符号链接、宽权限、用户不符、非本地文件系统或无法安全检查的目录直接拒绝，不掩盖攻击后回退。CLI/service/helper 使用同一解析结果；路径过长明确报错，需用户选择较短的有效显式运行目录。

`paths/doctor --json` 显示 `runtimeSource`、`runtimeLifecycle` 和诊断；替代目录的 warning 同时出现在 stderr 与 JSON `warnings`。这不是标准 `XDG_RUNTIME_DIR`，不保证 SIGKILL 或复杂多登录后立即清空。服务通过 UID、实例标识、socket、进程存活及 runtime/STATE 两把进程锁验证连接和恢复：同一运行目录或同一 STATE 只能有一个服务；只有显式隔离两者才能独立运行。正常退出移除自己的 socket/实例记录，保留稳定锁文件，避免等待者分裂锁；强杀重启只回收已确认失效且匹配用户/STATE 的资源，不删除活跃实例或未确认的 helper。

```sh
omm init
omm config show --json
omm config set capture_executable /absolute/path/OMMCapture.app/Contents/MacOS/omm-capture
omm config set worker_executable /absolute/path/omm-asr-worker
omm config set asr.library_path /absolute/path/libsherpa-onnx-c-api.dylib
omm config set pi.executable /absolute/path/pi
omm config set pi.provider YOUR_PROVIDER
omm config set pi.model YOUR_MODEL
omm config validate --json
```

`config set` 只接受列出的非秘密字段，不接受 API key/token/password。使用 Pi 自身支持的方式，由用户为 **OMM 专用** `CONFIG/omm/pi` 目录配置认证和模型；程序不继承全局 Pi 配置或真实 HOME，不复制全局凭据。飞书检测可配置 `lark_executable` 和 `rtc_log_dirs`；需要飞书 CLI 用户认证/API 权限以及与该用户参与状态匹配的新鲜本地会议日志。进程/窗口只是弱信号，不能单独自动录音；观察失败是 unknown，不是会议结束。一次观察的总预算为 8 秒，最多查询 8 个会议的参与状态、扫描 8 个日志目录；观察任务不占服务控制循环，停止时取消并回收其子进程。过期配置或手动控制之前发起的观察结果不再应用。

## 用户级安装与卸载

完整源码安装步骤及固定官方原生库包的 SHA-256 见[安装指南](docs/install.md)。不需要管理员权限：CLI 在 `~/.local/bin/omm`，worker/helper/native 库可放 `DATA/omm/program`，服务是用户 LaunchAgent。构建不会自行安装这些组件。Pi 仍需要它自己的 Node 运行时和专用认证；`pi.executable` 应显式指向受信稳定入口（例如 Pi 管理安装器的 `~/.pi/agent/bin/pi`），JavaScript launcher 可另配 `pi.node_executable` 的绝对路径，不继承个人 PATH。工具升级及旧任务恢复见[安装指南](docs/install.md#4-专用-pi-认证与系统权限)。OMM 不分发 Python 转写运行时，不自动更换模型/provider。

```sh
omm service install --json
omm service start --json
omm service status --json
omm service stop --json
omm service uninstall --json
```

安装是显式操作，`RunAtLoad` 可在安装及下次登录启动；`KeepAlive=false`，正常停止后不自动重启。卸载不删除配置、数据库、录音、模型或产物；移除命令入口及自有程序目录请见安装指南，不要递归删除整个 DATA。macOS 开发签名不等于公证，首次权限或签名升级可能需要原生桌面授权，不能冒用 Yulu 授权。

## 模型与授权是显式操作

```sh
omm model list --json
omm model download fire-red-asr-int8
omm model download silero-vad
omm model verify fire-red-asr-int8
omm permissions check --json
omm permissions request
```

模型 ID 以 `model list` 的编译清单为准。下载走 HTTPS，校验包大小、SHA-256、解压路径与逐文件 SHA-256，再原子安装到 DATA；不会下载或安装没有已验证清单的 native 动态库。库文件由用户明确定位，worker 检查运行时版本。下载不会改写配置；请用 `config set asr.encoder_path/decoder_path/tokens_path/vad_path` 分别填入 `model list` 中各文件的绝对路径（每次只设置一个字段）。移除模型用 `model remove <id> --confirm`，与正在转写的模型读取互斥。诊断/构建/测试不会自动下载约 1.74 GB 的真实权重。

权限请求只打开系统设置，不授予权限，不声称授权成功；非交互环境返回结构化 `interaction_required`。只读检查可能报告 unknown，helper 的准备状态不等于完整系统权限审计。真实录音仅由用户的明确开始命令或已同意的自动录音触发。

## 服务、录音与处理

```sh
omm service run --non-interactive
omm service status --json
omm record start --title '示例会议'
omm record status --json
omm record stop --json
omm record auto enable --confirm-recording-consent
omm record auto disable
omm meeting list --json
omm task list --json
```

前台与 LaunchAgent 服务均可使用上述显式目录或安全替代目录。`service install/start/stop/uninstall` 是用户显式操作；默认受控测试只使用临时 HOME 和伪服务运行器，不注册真实服务。安装时显式运行根随四个持久根记录到 profile/LaunchAgent；CLI 运行根冲突时拒绝操作。macOS 替代根不写为跨登录固定 locator，每次从系统重新解析；显式根确实消失时允许带诊断退化到替代根。`uninstall` 先尝试封存活跃录音，再注销服务，不删 DATA/STATE/CONFIG 或已提交结果；无法确认活跃录音已封存时拒绝卸载。封存成功后，对服务启动或接管的 helper 都执行身份绑定的退出请求，验证实际进程退出和 socket 消失后才报告成功；错误身份、仍在录制、退出失败或已接管端点异常丢失均拒绝成功。不杀未验证的 PID，不自行删除 capture socket；从未启动 helper 的服务可正常停止。

自动录音默认关闭。15 秒强信号确认、30 秒结束宽限是可配置建议，不是已验证的漏录指标；轮询可能漏掉开头或多录尾段。手动停止抑制当前会议自动重启；无法识别来源时需明确结束或再次手动开始才能解除保守抑制。服务重启先对账，不重复起录；同来源 ID 重连保留 session，不同强来源 ID 按稳定确认时间建立候选，先封存旧会议再开始独立会议。短抖动、unknown 和手动录制不触发身份切换。帧级轮换保留原 session，默认/最大 `1800` 秒；`omm config set segment_ms 60000` 可设置60000–1800000的整数毫秒。正常轮换只切已配对PCM的写入文件，不停止/重启采集。恢复或系统睡眠仍可能产生缺口。睡眠后的对账不能保证睡前通知、恢复完整性或无丢音，需要真实机器验证。

有效片段封存后，不等会议结束就进入独立持久转写队列；无音频帧不创建任务。每段以及已识别的语音活动段都有绑定输入/模型/版本的检查点，失败、超时或重启只补未完成部分。`meeting show` / `task show` 显示段状态和无正文的处理进度；`meeting read --artifact transcript` 在最终转写提交前返回 `partial:true`、已完成/总片段数和“尚未完成”提示；空内容表示尚无已提交片段，不能当成空白终稿。会议封存且所有段完成后，按采集帧位置聚合一次最终转写并排队调用 Pi，不生成每段纪要或每段启停通知。

码率默认48kbps，可用 `omm config set audio.opus_bitrate_kbps 64` 选择32/48/64/96；`omm config show` 显示当前值。每段正式入队时冻结码率，改配置只影响随后入队的新段；已入队但尚未编码的段也沿用原值。旧归档/检查点不重写，`process --force` 只重转写、不重新编码；仍有原WAV时使用校验后的原WAV，否则解码旧归档。输入记录区分当前请求值和原归档实际码率、编码器版本及hash。

采集仍使用已有PCM配对、同步、WAV写头与恢复内核。后台编码与ASR各有一条队列：最终公开混音和内部麦克风/系统声源各一份单声道 `.opus`，转写保留声源标签。归档检查容器、预跳过帧、尾填充、解码帧数和hash，原子发布收据；只有归档和段检查点验证完成、没有音频读者租约时才清理本次临时WAV。历史WAV永不自动清理，仍可处理。崩溃时合法零帧空尾保留原文件、排除逻辑段列表；截断或格式异常的尾文件保守报错。缺编码器或编码失败保留原WAV，并在归档进度中报告错误，不能当成音频归档已就绪。三份音频增加空间和编码成本，来源文件不在默认音频列表展示。

新配置转写默认3线程、单worker，已保存的2线程配置保持原值；按逻辑核数的一半预留采集/服务和单线程编码槽后限制1–3线程，小核机器共享编码/ASR槽。编码使用 `nice 10`、单线程/单过滤线程；没有动态采样控制算法。CPU目标是所属进程总CPU÷逻辑核数约低于50%，不是系统硬配额。基准与取舍见[性能记录](docs/segment-benchmark.md)。

音频只进入本地 ASR worker。逐字稿独立提交后自动排队调用 Pi，不存在文本外发批准开关；Pi 的 stdin 只含转写文本/时间信息，**不传音频、路径或任务数据库**。Pi 使用固定 print argv，禁工具、MCP、扩展、技能、上下文文件、会话与自动批准；长文本分块后合并。权限收缩不是操作系统沙箱，配置文件和供应商实现仍需信任。

Pi 临时目录使用实例/任务/attempt/STATE 所有权记录与独占锁。取消及正常完成先等待OMM持有句柄的进程退出并处理输出，再提交结果或宣布取消/发送通知；后台持有管道导致既有预算内未读到关闭时，明确记录未验证并保留目录。取消不发布新纪要、不自动重新生成。这个保证不覆盖任意启动脚本创建的未知后台进程。服务被强制终止后，孤儿可能继续调用模型，恢复不会凭PID终止它。每次尝试启动Pi前，持久记录后台未验证；主动清理、Drop及恢复均保留相应私有工作目录并诊断，旧记录也保守保留，不凭PID或空闲锁宣称完整退出、不盲杀未知进程。目录可能含文本并持续占用缓存，取消不承诺擦除文本；录音、旧纪要和检查点保留。见[安装与故障处理](docs/install.md#pi工作目录留存)。

```sh
omm meeting list --from 2026-10-01 --query '项目' --status completed --json
omm meeting show <meeting-id> --json
omm meeting read <meeting-id> --artifact transcript
omm meeting read <meeting-id> --artifact summary
omm process <meeting-id> --stage all --wait --timeout 20m
omm process --task <task-id> --retry
omm process <meeting-id> --stage summary --force
omm task show <task-id> --json
omm task wait <task-id> --timeout 20m --json
```

普通处理按输入和配置幂等；`all --force` 新建 attempt，保留旧产物。`summary --force` 只复用校验后的已提交逐字稿，记录来源 attempt/hash/原 ASR 配置，使用新的 Pi 配置生成纪要，不隐式重新转写。纪要失败只重试纪要；有限退避、租约续期和提交 journal 支持重启恢复。`wait` 超时不取消任务或删除音频。

`meeting list --query` 只匹配标题，`--source` 匹配来源 ID，不搜索录音或逐字稿全文；`read` 返回已提交或明确标记的部分文本，转写为结构化 JSON 内容；`meeting show` 的 `audio` 仅列公开混音归档。所有命令支持 `--json` 与 `--non-interactive`。退出码：0 成功；2 输入/配置；3 缺依赖、认证、权限或交互要求；4 损坏/真实处理错误；5 等待超时；6 冲突。后端失败诊断不回显会议内容、凭据或原始 stderr。

## 验证边界与许可

`cargo test --locked --all-targets` 使用临时目录、合成音频、受控 helper/Pi/worker 和 native C API 夹具。本机验收另用官方 sherpa-onnx 1.13.2 与已校验 FireRedASR/Silero，转写自行生成的27.7秒中英文双轨 WAV，再用禁用个人上下文/扩展/工具的 Pi、`openai-codex/gpt-6.1-sol` 对本次文本生成短纪要，均正常退出。双轨同内容的近场/系统音去重保留系统来源，该样本不证明真实硬件采集或长会议质量。真实 capture IPC 在无人桌面授权环境超时，具体权限仍 unknown，不能据此判定某项权限已被拒绝。真实系统授权、双源录音、正在进行的飞书个人会议检测、整小时质量/性能、macOS 13 实机、其他平台及发布都未验证。sherpa 的 int32 VAD 时间索引存在约 37.28 小时的 ABI 上限，超限明确失败，不宣称无限时长。

真实 Swift 的合成进程退出回归需显式指定开发二进制，不访问录音硬件：

```sh
OMM_CAPTURE_TEST_BINARY="$PWD/target/capture/OMMCapture.app/Contents/MacOS/omm-capture" cargo test --locked --test capture_adapter -- --ignored
OMM_CAPTURE_TEST_BINARY="$PWD/target/capture/OMMCapture.app/Contents/MacOS/omm-capture" cargo test --locked --test core_integration review_r1_03_ -- --ignored
```

项目源码采用 [Apache-2.0](LICENSE)，不改变依赖/模型的各自许可。Swift 内核与算法参考 Yulu 的 MIT 许可保留于 [录音许可](native/yulu-capture/LICENSE)、[录音来源及适配说明](native/yulu-capture/NOTICE.md)、[转写参考许可](workers/local-asr/LICENSE-Yulu)。sherpa 官方头文件的许可见 [LICENSE-sherpa-onnx](workers/local-asr/LICENSE-sherpa-onnx)。编码工具固定 FFmpeg 8.1.2（关闭GPL/网络/自动探测，LGPL-2.1-or-later）及 libopus 1.5.2（BSD-3-Clause）；源码hash与许可见 [编码工具清单](packaging/audio/sources.json) 和 [许可目录](packaging/audio/licenses)。显式用户级安装会保留完整校验源码和重建脚本。CLI开发构建不复制模型/动态库。桌面release包另按固定清单携带CLI/worker/helper、sherpa/ONNX Runtime、codec及各自许可/来源；模型权重仍由hash清单和XDG目录管理，不随App暗中下载。实际再分发以当前包资源和许可证验证为准。

## 桌面状态通知

通知默认开启：真实开始录音、封存并排队转写、纪要提交完成，以及简短阶段错误；显示可信标题（可关闭）、开始时间/时长和安全原因，不显示任务ID或正文，轮换/重连不重复刷屏，失败不影响处理结果。macOS 使用系统 AppleScript 宿主，不新增独立 OMM 通知权限身份。`omm notifications check --json` 报可用机制及 unknown 权限；`omm notifications request` 仅打开设置；`omm notifications test --json` 显式发送一条无敏感测试。可用 `omm config set notifications.enabled false` 关闭。失败按阶段共享60秒持久冷却，同一 task/attempt 重试在冷却结束后可再次提醒；通知拒绝不改变业务结果。队列取消有3秒预算，但确认自有子进程退出可能更久。事件筛选、宿主权限身份和安全部署步骤见[通知说明](docs/install.md#7-桌面状态通知)。调用成功与用户实际看见分别验证；后台可见递送及 macOS 13 全链兼容仍待桌面实测。此变化不默认开启自动录音，也不覆盖已有 auto/consent 配置；运行中的会议或任务不能为通知测试被中断。
