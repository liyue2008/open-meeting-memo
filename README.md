# Open Meeting Memo (OMM)

本地录音和转写，使用已配置的 Pi 生成会议纪要。首版面向 macOS、Apple Silicon、飞书与命令行；录音 helper 编译目标是 macOS 13+，本指南固定的官方 ASR 库依赖 macOS 15.5+，较旧系统未验证完整链路。不包含实时字幕、说话人识别、网页界面或 MCP 服务。

**当前为开发版本，不是已发布或验证过真实会议的产品。** 既有86项受控基线已通过独立代码复审；本次新增启动恢复修复待独立复审。目前80项默认测试加11项真实 Swift 合成回归通过；它们不代表真实录音、系统授权或模型质量验收。Swift 录音适配器、Rust 原生转写 worker、核心队列及 CLI 已接线。

- [用户级安装、依赖、卸载与故障处理](docs/install.md)
- [贡献者/Agent 指南](AGENTS.md) · [概要设计及边界](docs/overview-design.md) · [需求](prd.md)

用户已接受 macOS 安全替代运行目录与最佳努力清理：没有可用的显式 `XDG_RUNTIME_DIR` 时使用系统用户临时区域，不把 STATE/CACHE 冒充运行目录，也不承诺完整登录生命周期管理。服务注册成功不表示后台或权限已就绪；不要把单测或开发构建当作实际授权、录音、真实模型推理或发布证明。

## 构建

需要已有的 Rust、Xcode/Swift 工具链，不由构建脚本安装全局工具。

```sh
cargo build --locked
bash packaging/capture/build-dev.sh
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

完整源码安装步骤及固定官方原生库包的 SHA-256 见[安装指南](docs/install.md)。不需要管理员权限：CLI 在 `~/.local/bin/omm`，worker/helper/native 库可放 `DATA/omm/program`，服务是用户 LaunchAgent。构建不会自行安装这些组件。Pi 仍需要它自己的 Node 运行时和专用认证；OMM 不分发 Python 转写运行时，不自动更换模型/provider。

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

自动录音默认关闭。15 秒强信号确认、30 秒结束宽限是可配置建议，不是已验证的漏录指标；轮询可能漏掉开头或多录尾段。手动停止抑制当前会议自动重启；无法识别来源时需明确结束或再次手动开始才能解除保守抑制。服务重启先对账，不重复起录；同来源 ID 重连保留 session，不同强来源 ID 按稳定确认时间建立候选，先封存旧会议再开始独立会议。短抖动、unknown 和手动录制不触发身份切换。轮换音频段保留原 session 和缺口记录。睡眠后的对账不能保证睡前通知、恢复完整性或无丢音，需要真实机器验证。

封存音频只进入本地 ASR worker。逐字稿独立提交后自动排队调用 Pi，不存在文本外发批准开关；Pi 的 stdin 只含转写文本/时间信息，**不传音频、路径或任务数据库**。Pi 使用固定 print argv，禁工具、MCP、扩展、技能、上下文文件、会话与自动批准；长文本分块后合并。权限收缩不是操作系统沙箱，配置文件和供应商实现仍需信任。

Pi 临时目录使用实例/任务/attempt/STATE 所有权记录与独占锁。正常取消先终止并等待持有句柄的子进程退出，再删除自己的目录；服务正常停止等待回收完成。父服务被 SIGKILL 时 Pi 可能成为孤儿并继续模型调用，不保证立刻终止。恢复仅在所有权/任务匹配、锁空闲且父与子 PID 均确认死亡后删除；活跃、身份/存活未知、损坏记录或路径攻击保留并诊断，不凭 PID 杀进程，也不删除其他 STATE 的目录。

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

`meeting list --query` 只匹配标题，`--source` 匹配来源 ID，不搜索录音或逐字稿全文；`read` 返回已提交文本，转写为结构化 JSON 内容。所有命令支持 `--json` 与 `--non-interactive`。退出码：0 成功；2 输入/配置；3 缺依赖、认证、权限或交互要求；4 损坏/真实处理错误；5 等待超时；6 冲突。后端失败诊断不回显会议内容、凭据或原始 stderr。

## 验证边界与许可

`cargo test --locked --all-targets` 使用临时目录、合成音频、受控 helper/Pi/worker 和 native C API 夹具。本机验收另用官方 sherpa-onnx 1.13.2 与已校验 FireRedASR/Silero，转写自行生成的27.7秒中英文双轨 WAV，再用禁用个人上下文/扩展/工具的 Pi、`openai-codex/gpt-6.1-sol` 对本次文本生成短纪要，均正常退出。双轨同内容的近场/系统音去重保留系统来源，该样本不证明真实硬件采集或长会议质量。真实 capture IPC 在无人桌面授权环境超时，具体权限仍 unknown，不能据此判定某项权限已被拒绝。真实系统授权、双源录音、正在进行的飞书个人会议检测、整小时质量/性能、macOS 13 实机、其他平台及发布都未验证。sherpa 的 int32 VAD 时间索引存在约 37.28 小时的 ABI 上限，超限明确失败，不宣称无限时长。

真实 Swift 的合成进程退出回归需显式指定开发二进制，不访问录音硬件：

```sh
OMM_CAPTURE_TEST_BINARY="$PWD/target/capture/OMMCapture.app/Contents/MacOS/omm-capture" cargo test --locked --test capture_adapter -- --ignored
OMM_CAPTURE_TEST_BINARY="$PWD/target/capture/OMMCapture.app/Contents/MacOS/omm-capture" cargo test --locked --test core_integration review_r1_03_ -- --ignored
```

项目源码采用 [Apache-2.0](LICENSE)，不改变依赖/模型的各自许可。Swift 内核与算法参考 Yulu 的 MIT 许可保留于 [录音许可](native/yulu-capture/LICENSE)、[录音来源及适配说明](native/yulu-capture/NOTICE.md)、[转写参考许可](workers/local-asr/LICENSE-Yulu)。sherpa 官方头文件的许可见 [LICENSE-sherpa-onnx](workers/local-asr/LICENSE-sherpa-onnx)。模型/动态库不随开发构建打包，重新分发时须另外核对其许可证。
