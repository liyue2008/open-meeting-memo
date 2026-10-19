# 用户级安装、依赖与故障处理

这是源码开发安装，不是签名公证的发行包。首版目标是 macOS / Apple Silicon；录音 helper 编译目标为 macOS 13+，但**本指南固定的原生库所带 ONNX Runtime 1.24.4 要求 macOS 15.5+**，不能据此宣称完整链路兼容 macOS 13。较旧系统需另行验证兼容的同版库构建；Intel 及 Windows/Linux 尚未实机验收。所有步骤用普通用户执行，无 sudo、系统服务或驱动。

## 桌面 App 状态

Tauri 2 桌面 App 面向 Apple Silicon/macOS 15.5+，真实前端和原生组件已合入整合分支。统一入口及当前验证状态见[桌面开发](desktop.md)和[验收矩阵](desktop-acceptance.md)。以下 CLI 源码安装流程仍可独立使用，不能代替桌面产品验收。

桌面构建需要已有 Rust/Xcode、Node 22.19–22.x、npm 11.x；构建不会安装系统工具。干净已提交源码可执行：

```sh
bash scripts/test-desktop.sh --wkwebview
bash scripts/package-desktop.sh
# 用脚本打印的完整 .app 路径替换下方参数；安装必须另行显式执行
bash scripts/install-desktop.sh --check
bash scripts/install-desktop.sh --from "/absolute/path/to/Open Meeting Memo.app"
```

桌面包包含自有 CLI、worker、Swift capture helper、固定 sherpa/ONNX Runtime 与 codec 闭包、许可证和源码材料；模型权重仍按固定 manifest 安装至 XDG DATA，Pi/Node/Lark 是外部集成。安装器先验证资源/hash/签名与已有安装元数据，发布到 `~/Applications/Open Meeting Memo <version>-<revision前12位>.app`，保留旧版本；不覆盖 CLI/LaunchAgent，不改配置、录音、认证、auto/consent，也不启动个人服务。具体 receipt 和位置以安装器输出及[原生包装说明](desktop-native.md)为准。

正常无参数启动仍使用原有持久WebKit偏好。只想隔离检查生产App启动时，可显式选择本次非持久模式；不是fixture宿主，也不会初始化profile或启动服务。先用已验证安装路径设置`APP`，再在全新私有目录清继承环境运行：

```sh
APP="/absolute/path/to/installed/Open Meeting Memo <version>-<revision>.app"
T=$(mktemp -d /private/tmp/omm-production-check-XXXXXX)
chmod 700 "$T"
mkdir -m 700 "$T"/{home,config,data,state,cache,runtime,tmp}
env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin LANG=en_US.UTF-8 \
  HOME="$T/home" TMPDIR="$T/tmp" XDG_CONFIG_HOME="$T/config" \
  XDG_DATA_HOME="$T/data" XDG_STATE_HOME="$T/state" \
  XDG_CACHE_HOME="$T/cache" XDG_RUNTIME_DIR="$T/runtime" \
  "$APP/Contents/MacOS/omm-desktop" --ephemeral-webview --webview-store-diagnostics
```

诊断只输出实际main视图存储的非内容JSON（`persistent=false`）；参数/平台不支持时拒绝，不回退default store。不要传fixture变量或store manifest，HOME/XDG本身不是WebKit隔离证明。该检查可能只创建空0600的service.lock，不代表配置/数据库已初始化；不请求录音权限或自动开始工作。关闭并确认明确owned进程退出后再处理私有检查目录，不清个人WebKit数据。普通生产持久偏好与两进程保持不受此显式模式影响。

首次打开仅只读观察。首次初始化、启动/停止/安全切换服务均需在“状态与诊断”显式确认；旧或未知服务/不兼容 schema、活动录音或未封存数据会拒绝不安全写入。缺 owner 标记表示待核验采用，不直接断言 schema 不兼容；严格受支持的停机旧资料库采用成功后仍停止，须另点启动，保留原配置、auto/consent、媒体和个人 CLI。不可验证或仍加载的旧服务不由 App 强制控制；详见[服务恢复与安全人工步骤](desktop-service-recovery.md)。模型、系统权限与专用 Pi 认证分别诊断，服务运行不冒充这些依赖已就绪。开发签名是 ad-hoc，不是 Apple 公证。

后续 release 安装只进入当前用户目录，先只读检查安装元数据、备份非秘密元数据，保留所有录音/配置/认证/auto/consent，不打断在录会议、不盲杀未知进程、不因启动 App 迁移旧 profile。可安全并存版本化 App，并以实际生产包＋私有 profile 验证窗口和真实桥；个人旧 daemon 不兼容时准确报告显式切换路径。ad-hoc 签名不是公证，实际注册也不是权限/运行就绪。

## 1. 构建及复制到稳定位置

需要已有 Rust、Xcode/Swift 工具链；发布构建及模型解包需留出数 GB 额外磁盘空间，安装后模型约 1.74GB，CPU 推理还需额外内存。Pi 需要它自己的 Node 运行时（当前 Pi 1.1.0 文档要求 Node 22.19+）。[Pi 官方项目](https://github.com/badlogic/pi-mono/tree/main/packages/coding-agent) 的安装/认证方式可能随版本改变，请核对本机 `pi --version`、`pi --help`，不要使用不明安装脚本。

```sh
cargo build --release --locked
bash packaging/capture/build-dev.sh
/usr/bin/codesign --verify --strict target/capture/OMMCapture.app
```

复制前检查目标是否已有文件。下例仅用于**空目标**；升级先停止服务，备份非秘密配置/安装元数据，不复制认证文件或覆盖会议数据。不要在服务运行中替换二进制。

```sh
PREFIX="$HOME/.local/share/omm/program"
mkdir -p "$HOME/.local/bin" "$PREFIX/bin" "$PREFIX/capture"
chmod 700 "$HOME/.local/share/omm" "$PREFIX" "$PREFIX/bin" "$PREFIX/capture"
install -m 700 target/release/omm "$HOME/.local/bin/omm"
install -m 700 target/release/omm-asr-worker "$PREFIX/bin/omm-asr-worker"
/usr/bin/ditto target/capture/OMMCapture.app "$PREFIX/capture/OMMCapture.app"
/usr/bin/codesign --verify --strict "$PREFIX/capture/OMMCapture.app"
"$HOME/.local/bin/omm" init
```

可在当前终端 `export PATH="$HOME/.local/bin:$PATH"`，不必修改 shell 配置。自有 bundle ID 为 `org.openmeetingmemo.capture`；ad-hoc 签名无需个人证书，但源码更新可能改变系统权限身份，不能继承 Yulu 授权。

## 2. 原生转写库：固定官方包，不用测试夹具

OMM 精确要求 sherpa-onnx **1.13.2**，动态加载 C API。可用的官方 Apple Silicon 无 TTS 库包（不需要语音合成组件）：

- [官方版本/资产](https://github.com/k2-fsa/sherpa-onnx/releases/tag/v1.13.2)
- 资产 `sherpa-onnx-v1.13.2-osx-arm64-shared-no-tts-lib.tar.bz2`，14,348,214 bytes。
- SHA-256 `949bfacca8cfd9fe80d7067a27fdd2d44f874e7b8ef99736d73af609891784e6`（官方 GitHub release 元数据）。

```sh
NATIVE="$HOME/.local/share/omm/program/native"
ARCHIVE="$HOME/.cache/omm/sherpa-onnx-v1.13.2-osx-arm64-shared-no-tts-lib.tar.bz2"
mkdir -p "$NATIVE" "$HOME/.cache/omm"
chmod 700 "$NATIVE" "$HOME/.cache/omm"
curl --fail --location --proto '=https' --tlsv1.2 \
  'https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.2/sherpa-onnx-v1.13.2-osx-arm64-shared-no-tts-lib.tar.bz2' -o "$ARCHIVE"
printf '%s  %s\n' '949bfacca8cfd9fe80d7067a27fdd2d44f874e7b8ef99736d73af609891784e6' "$ARCHIVE" | shasum -a 256 -c -
tar -tjf "$ARCHIVE"
# 确认清单没有绝对路径/路径穿越，再解到空目录；保留完整包及依赖的相对布局。
tar -xjf "$ARCHIVE" -C "$NATIVE"
```

安装后用 `file`、`otool -L` 检查 `libsherpa-onnx-c-api.dylib` 为 arm64，依赖在同一库目录可解析；保留库的链接和 `@rpath` 布局，不只复制一个 dylib，不用 `DYLD_LIBRARY_PATH` 绕过布局错误。版本字符串检查不是完整性证明，完整包哈希才固定所装字节。库包/依赖的最低系统版本须单独检查，不由 Swift 的 macOS 13 编译目标推定。

sherpa-onnx 使用 [Apache-2.0](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.2/LICENSE)，ONNX Runtime 使用 [MIT](https://github.com/microsoft/onnxruntime/blob/main/LICENSE)。本项目不分发这些二进制；如重分发须携带其许可及第三方 notices。

## 2a. 分段录音的 Ogg/Opus 编码工具

最终音频不是改名的WAV。需要已有 `clang`、`make`、`pkg-config`、`curl`、`tar` 和Xcode签名工具；不要用sudo、全局brew安装或复用启用了GPL的本机FFmpeg。脚本只在仓库 `target/audio-toolchain` 构建，显式下载以下官方源码并逐个校验：

|源码|SHA-256|许可|
|---|---|---|
|[FFmpeg 8.1.2](https://ffmpeg.org/releases/ffmpeg-8.1.2.tar.xz)|`464beb5e7bf0c311e68b45ae2f04e9cc2af88851abb4082231742a74d97b524c`|LGPL-2.1-or-later|
|[libopus 1.5.2](https://downloads.xiph.org/releases/opus/opus-1.5.2.tar.gz)|`65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1`|BSD-3-Clause|

关闭GPL、version3、网络与自动探测，只启用本地WAV/Ogg、Opus/PCM及所需过滤器；编译后开发签名并记录binary hash。完整源码包、许可、清单和重建脚本伴随安装，用于用户修改/重建，不将本机GPL shared binary作为可再分发包。

```sh
bash packaging/audio/build-dev.sh
# 安装/升级前正常停止服务；在途录音先封存、已拥有子进程先等待退出
omm service stop --json
omm codec install --from "$(pwd -P)/target/audio-toolchain" --json
omm codec check --json
omm service start --json
```

安装到 `DATA/omm/audio-toolchain`，不更改用户配置/认证，不启动录音或申请桌面权限。CLI校验整个包的版本、官方源码hash、binary hash、许可、当前用户及路径/权限，私有原子安装；读者/工作仍在途则拒绝替换。服务启动时加载编码工具，因此安装后需重启。`codec check` 和 `doctor.audioCodec` 的ready只表示依赖验证成功，不表示录音权限或可见通知已就绪。

如需从已安装源码重建，将保留的 `build-dev.sh` / `sources.json` 放进一个空目录的 `packaging/audio/`，将两份源码包放进该空目录的 `target/audio-toolchain/sources/`，再显式执行 `bash packaging/audio/build-dev.sh`；脚本优先使用已有校验包，可离线重建。重分发保留这些源码、脚本、许可与配置参数。

缺工具或编码失败时保留内部WAV，转写队列独立继续；`meeting show` / `task show` 的 `segmentProgress.archives` 显示归档状态/错误。修复依赖并重启后，失败段/归档使用 `omm process --task <父task-id> --retry`；有效段检查点不重做，已完成会议不会再调用Pi。公开 `audio` 只列混音 `.opus`，内部麦克风/系统声源供后续重转使用。

## 3. 模型与明确路径配置

模型 manifest 在 [packaging/asr/models.json](../packaging/asr/models.json)，约 1.74GB 已安装权重。下载不修改配置；包及各文件均校验后才安装。模型只在本地推理，不以其他模型代替下载失败。

```sh
omm model list --json
omm model download fire-red-asr-int8
omm model download silero-vad
omm model verify fire-red-asr-int8 --json
omm model verify silero-vad --json
omm config set capture_executable "$HOME/.local/share/omm/program/capture/OMMCapture.app/Contents/MacOS/omm-capture"
omm config set worker_executable "$HOME/.local/share/omm/program/bin/omm-asr-worker"
```

下面路径占位符需替换为解包位置及 `model list` 返回的绝对路径，每次设置一个字段；不能原样运行尖括号内容。

```sh
omm config set asr.library_path '<absolute lib/libsherpa-onnx-c-api.dylib>'
omm config set asr.encoder_path '<absolute fire-red-asr-int8/encoder.int8.onnx>'
omm config set asr.decoder_path '<absolute fire-red-asr-int8/decoder.int8.onnx>'
omm config set asr.tokens_path '<absolute fire-red-asr-int8/tokens.txt>'
omm config set asr.vad_path '<absolute silero-vad/silero_vad.onnx>'
omm config set pi.executable '<absolute installed pi executable>'
omm config set pi.provider '<provider available in your Pi installation>'
omm config set pi.model '<model available to your authorized account>'
omm config validate --json
```

FireRedASR 上游 [模型/许可说明](https://github.com/FireRedTeam/FireRedASR) 与 [sherpa 模型说明](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/fire-red-asr-models.html)；Silero 上游 [MIT 许可](https://github.com/snakers4/silero-vad/blob/master/LICENSE)。代码许可不自动代表训练数据或权重的全部使用限制，请在重分发前核对上游说明。

## 4. 专用 Pi 认证与系统权限

`CONFIG/omm/pi` 是专用 Pi agent 目录，OMM 总结调用不继承个人扩展/技能/工具/上下文。用 `PI_CODING_AGENT_DIR="$HOME/.config/omm/pi" pi`（自定义 XDG 时替换实际 CONFIG 路径）在 Pi 内按官方方式认证；认证文件设 0600、目录0700，不放进仓库/普通 OMM 配置。已有授权可使用 Pi 官方支持的安全认证机制引用；不要复制整个个人 agent 目录。Pi 1.1.0 也支持专用 `models.json` 中的自定义 provider endpoint/headers 使用 `!command` 读取授权：命令只返回所需字段、使用固定路径，不把 token 写进新文件或日志。OAuth endpoint 及额外账户 header 必须按已安装 Pi provider 实现核对；本方法不负责刷新过期授权，过期时仍需原生认证。不要打印完整认证内容、备份秘密或盲目继承原 `models.json`。OMM 不继承 shell API-key 环境变量，所选模型必须在专用目录可用。

### Pi 升级、稳定入口与已有结果恢复

`pi.executable` 设为用户明确选择的稳定入口，例如管理安装器的 `~/.pi/agent/bin/pi`，不要长期引用某个可删除的 npm/nvm 版本目录。支持用户或系统所有、不可被组/其他用户写入的可执行文件及受信符号链接。Pi和Node均按实际访问顺序检查每个中间目录/链接，展开相对或绝对目标后再解释`.`/`..`；root所有的sticky共享系统目录可用。单条路径/链接目标限4096字节、最多256步/40链接，循环和超限安全失败。最终目标安全也不能跳过0777/组可写中间目录。校验到spawn之间仍存在同一用户或管理员改路径的竞态，不是操作系统沙箱或固定inode执行保证；不重建旧版本目录来掩盖失效路径。JavaScript launcher 还需显式 `omm config set pi.node_executable '<受信 node 可执行文件绝对路径>'`；清除此可选项用 `omm config set pi.node_executable null`。Node 文件名须为`node`，父目录必须能由原生PATH无损表示；macOS含冒号的父目录会在启动前脱敏拒绝。Pi入口自身仍按完整路径直接执行，可含冒号。不自动搜索个人 PATH，不改模型/provider，不删除隔离参数来兼容不支持的工具。OMM 保留隔离 HOME、专用认证目录与禁扩展/工具/上下文参数；管理安装器依赖真实 HOME 寻找 Node 时，必须单独配置受信运行时。Node 同样应选维护的稳定入口；若显式使用某个 nvm 版本，删除该版本前必须审计并更新工具引用。

工具缺失/启动失败、运行时缺失和不支持固定隔离参数均返回脱敏、不可自动重试的可操作原因；不能把它们当成 ASR 失败。修改全局 Pi 路径只影响随后创建的快照，**不会替换历史或运行中任务快照**。已提交转写始终可用`meeting read <meeting-id> --artifact transcript`读取。纪要报`artifact not committed`表示没有通过提交校验的纪要；先看`task show`的stage/status/error。转写返回`partial:true`时尚未完成，仅包含已完成片段，零个完成片段会返回空segments及明确提示，不拿语音块检查点冒充终稿。

恢复单场已有纪要：先确认终稿已提交/hash有效，原provider/model/prompt及输入未变，只更新受信工具路径（及必要Node引用），再显式执行`omm process <meeting-id> --stage summary --force`。它创建新attempt，严格复用已有转写和输入血缘，不重跑ASR，旧attempt及产物保留。每次force都是一次明确新请求，不要循环调用。仍在ASR的会议保持运行，完成终稿后再执行；若旧快照先因missing Pi blocked，记录事实后按此单次恢复。不要就地改数据库/旧快照，也不要全库retry。

`omm config set` 可编辑路径、模型、线程、超时、Pi 输入输出预算、轮询/确认/分段时长和 `rtc_log_dirs`（JSON 数组）；完整字段见 [AppConfig::set](../src/core/config.rs)。`config show` 故意只返回非秘密摘要，`config validate` 只校验结构。`doctor` 的成功退出和 `asrReady` 不证明实际推理，权限/认证 unknown 不等于已授权。

```sh
omm paths --json
omm doctor --json
omm permissions check --json
omm permissions request
```

权限请求仅打开系统设置。需要用户在原生界面批准 OMMCapture 的麦克风、系统/屏幕音频，窗口读取还可能需要辅助功能。无人交互时返回 blocked/unknown，不使用权限数据库修改、模拟点击或管理员命令。全系统轨可能包含其他应用声音，短测试前关闭无关播放。

## 5. 后台服务与自动流程

```sh
omm service install --json   # 注册并 RunAtLoad；不代表就绪
omm service start --json
omm service status --json
omm record start --title '自行生成的测试会议' --json
# 等待短时受控录音，再封存；封存后自动本地转写 → Pi 纪要
omm record stop --json
omm task list --json
omm service stop --json
```

LaunchAgent 位于 `~/Library/LaunchAgents/org.openmeetingmemo.service.plist`，不需要管理员。`KeepAlive=false`，服务停止后不会自动重启；下次登录 `RunAtLoad` 会启动。安装固定四个持久 XDG 根；更改这些根前先卸载服务，不能通过切换 shell 环境偷偷启第二套数据库。

服务采用 `ProcessType=Standard`。Apple 本机 [launchd.plist(5)](x-man-page://5/launchd.plist) 定义 Standard 等同省略 ProcessType，使用普通的轻度资源限制；Background 会采用更保守的后台限制。服务仍是无窗口的用户 LaunchAgent；单ASR worker、已有显式2线程和静态核数预算不变，编码器仍 nice10、单线程。无需 Interactive、实时优先级或动态CPU控制。Adaptive 依赖XPC活动切换类别，不适合这里的常驻socket服务。受控20秒对照发现同模型/线程的旧Background显著慢于普通进程；真实长会的收益仍需部署后观察。

升级旧Background注册时，按下列顺序操作；stop会封存录音并等待helper/已拥有子进程退出，成功后才替换二进制并install：

```sh
omm service stop --json      # 失败时停止升级，不强杀或忽略封存错误
# 仅stop成功后：按第1节检查并替换自有CLI/worker二进制
omm service install --json   # 验证旧注册；bootout旧job、原子换Standard、bootstrap
```

同身份Standard重复install不改文件或重启。旧Background只接纳与OMM生成模板逐字一致的注册：同当前用户、canonical普通文件/目录、可信可执行路径、label/固定argv/profile/XDG环境；未知自定义字段（包括额外环境变量）、根/可执行路径变更或不安全文件均拒绝，保留原件。已有服务仍活跃、录音未封存或helper端点残留时拒绝升级。升级仅改变ProcessType，不改auto、同意状态、Lark/Pi认证引用或用户配置；RunAtLoad可能按用户既有auto策略恢复工作。仅编辑plist文件不能改变已加载进程的调度类别。

install/start/unregister/uninstall共用 `~/Library/LaunchAgents/.org.openmeetingmemo.service.registration.lock`（0600，非`.plist`）；文件持久保留，不能删除重建。锁按同一canonical HOME/固定label共享，不随XDG根变化；遇并发立即报Conflict，待原操作结束后显式重试。锁覆盖bootstrap和回滚；runtime锁在bootstrap前释放，daemon不获取注册锁。已有LaunchAgents父目录可为0755，不改其权限，但拒绝非本人所有、symlink或group/other可写目录。

录音封存检查还以只读方式查询独立的 `segmented_sessions.sealed=0`，不能只看录音intent；新注册和旧注册重载均拒绝未封存会话。缺少该表的旧库不迁移、不写状态；损坏数据库/异常seal schema报错。同身份Standard不重载分支仅复核身份，不扰动在录会话。

发布和回滚保留父目录FD并重验目录/目标inode、owner、mode和原字节；写临时0600文件并fsync后，相对该目录原子发布。等待bootout/bootstrap期间外部换文件、symlink、目录或原位修改时保留新文件和旧backup，报错要求检查，不盲目恢复/删除。同用户最后一次检查与rename之间仍是窄竞态窗口，不宣称跨用户提权防护或原子compare-and-swap。

升级保留非秘密旧模板 `~/Library/LaunchAgents/org.openmeetingmemo.service.pre-standard.backup`（0600，非`.plist`，不自动加载），仅含HOME/XDG/profile等已有路径；不备份任意环境秘密或认证文件。bootout失败不替换原注册；新bootstrap失败且目标仍是本次发布的身份/内容时，恢复旧模板并尝试重新注册旧job；目标有变化则不覆盖，恢复失败明确报错并保留backup。任何成功注册都不表示实际就绪。手工恢复也必须先正常封存/停止，再检查并还原自有backup、重新bootstrap；不能对活跃录音强制bootout。

长期默认 auto 关闭，不录制个人会议；需要时显式启用：

```sh
omm config set lark_executable '<absolute trusted lark-cli>'
omm config set rtc_log_dirs '["/absolute/authorized/rtc/log/directory"]'
omm record auto enable --confirm-recording-consent
omm record auto disable
```

自动录制要求新鲜、相互匹配的会议/本人参会/本地 RTC 强信号，进程/窗口不足。不要为了检测测试查询个人会议或整段飞书聊天记录。候选确认/结束宽限可能漏录开头或延长结尾。服务正常 stop 会封存并确认 helper 退出，Pi 子进程取消后等待回收；SIGKILL 不能保证同样清理。

### 分段、进度与CPU

`segment_ms` 默认1800000，单位整数毫秒，可设置60000–1800000；旧默认3600000只在加载时内存裁剪并告警，不自动改写配置；其他越界值拒绝。边界按帧切PCM写入，不stop/start采集。首个非空封存段即入持久队列，后续段可在会议录制中转写；最后尾段在停止/卸载时封存。重连/睡眠/崩溃的硬件连续性仍待实测。

```sh
omm config set segment_ms 1800000
omm config set asr.threads 3
omm meeting show <meeting-id> --json
omm task show <父task-id> --json
omm meeting read <meeting-id> --artifact transcript --json
```

码率默认48kbps，公开配置只接受32/48/64/96：`omm config set audio.opus_bitrate_kbps 64`，`omm config show` 查看当前值。每段入队时冻结；改值不重编码已入队/已归档音频，重启和重试沿用冻结值。`process --force` 只重新转写，优先使用仍保留的原WAV；否则从原归档解码，记录请求码率与原归档实际码率/编码器版本/hash。

部分转写标记 `partial:true`；进度只含声源/语音块数/语音秒数/处理时间，不含正文。结束后按原始帧位置聚合一份完整转写和一次纪要，不每段刷启停通知。每段及已识别语音块绑定输入/模型/配置/版本检查点；超时、失败、重启补做未完成部分。旧WAV/旧任务仍能恢复，只读命令不迁移旧数据库；显式写操作添加兼容表。新临时WAV还须有正常rolling封存的`temporary:true`凭证，归档/检查点均校验且无读者后才清理；中断恢复原件（包括合法零帧空尾）保留；零帧尾不算会议片段，截断/异常尾仍报错。恢复descriptor的`legacy_wav`须为布尔值且与来源一致；非temporary或legacy源不清理。单个归档清理失败持久标记该归档失败，后台队列继续后续任务；不能当成已清理成功。避免把升级前录音当成临时文件。旧用户录音永不自动清理。不要删除WAV来“修复”归档失败。封存descriptor兼容明确可选的`capture_clock` v1 typed字段；无字段的旧helper仍可读，present-null、未知字段/版本及非法数值会保留原件并报错，不放宽身份、descriptor/source hash、三份Ogg/Opus收据或读者锁校验。已完整编码但清理失败的归档，先确认该场所有转写片段done、收据/hash完整，在已完成的安全summary恢复attempt上显式`omm process --task <该场已完成attempt-id> --retry`；它只重排该场失败子任务。归档lane复用保留的完整收据再验证/清理，不重新编码；不完整/损坏收据仍保留WAV并报错。不要对仍带旧Pi坏路径的blocked attempt盲retry，也不要对尚有失败ASR段的会议借此要求“仅归档恢复”。

已结束且全部片段转写完成的会议由纪要队列先轻量聚合终稿、再生成一次纪要，不等待其他会议的音频转写队列清空；活跃会议、未完成片段及旧整段WAV仍遵循原规则，音频ASR不增加并行。

采集入口用实际源格式和持续的AVAudioConverter适配48kHz，再交给原PCM配对/WAV内核；process tap在非实时build/rebuild读取并验证aggregate输入流格式，IOProc只使用本代不可变快照，不逐包查询HAL。格式监听事件先停用旧代，再在非实时路径重建；已排队的旧回调持有旧context但不能再入队。格式读取/验证或回调布局失败会变为not-ready并返回固定`capture_source_format_*`/`capture_source_buffer_format_invalid`原因，不冒充TCC拒绝；没有任何有效音频时也不返回封存成功。ScreenCaptureKit路径直接校验每个sample buffer的实际格式。停止时排空转换器滤波尾，只裁剪转换器产生的尾部padding，不补静音。44.1kHz合成30分钟测试最终帧误差≤1帧，测试中的流式尾部≤512个48kHz帧（约10.7ms），不作为所有硬件格式的保证。helper的可选`capture_clock`只含源采样率、已接收帧/目标帧、声道数、格式拒绝数、采样率变化数和微秒单调时钟偏移；`writer_started_utc_ms`是WAV建立时刻，不冒充硬件确认时刻。旧descriptor没有这些字段仍可恢复。起止墙钟含检测/启动/停止延迟，不能单凭音长差断言丢音或改写历史录音；修复仅影响后续采集。

`task show --json` 的 `asrMeasurements` 提供最多最近256条数值测量（每条序列化不超过1KiB）和固定大小累计计数。每条绑定原任务attempt/lease、输入序号、mic/system、core及含padding的16k采样坐标；`native_decode_total` 的wall/进程CPU、cache查找与检查点写入使用微秒，UTC只用于关联，已有准备/校验/模型加载字段仍用毫秒。多线程进程CPU可以大于wall；该数字不能分开推断encoder/decoder耗时。`cache_hit` 本次decode/CPU/write为0，不复用旧decode成本；加载模型、校验及读取缓存仍有开销。token_count只读固定C API的结果token条数（未知/越界为null），不是EOS/停止原因；上游6token/秒上限仅是估计，不能据token数单判异常。测量不包含文字、token数组、标题、路径或原始错误，不发逐块通知；拒写、坏诊断或失效租约均不改变业务结果。旧worker/旧数据库可读且只读查询不迁移；v2检查点/key、模型/线程/timeout策略不变。这是取证工具，不代表慢转写已修复。

新配置默认单ASR worker、3线程，已保存的2线程配置保持原值；按半机逻辑核预算限制最多1–3；单编码器 nice10、threads/filter_threads各1，小核机器共享ASR/编码槽。没有动态CPU控制。macOS `ps` 的100%代表一个核，整机目标计算是**所属服务/capture/encoder/ASR进程CPU总和÷逻辑核数**，不是只看worker，也不包含无关应用负载；约低于50%为静态目标。可用活动监视器或 `ps -p <已确认所属PID列表> -o pid=,%cpu=` 采样求和，未知PID不要杀。短样本结果和速度取舍见[性能记录](segment-benchmark.md)，没有证实长会质量或硬件采集CPU。

## 6. 卸载与常见故障

```sh
omm record auto disable
omm service uninstall --json
```

卸载先封存、确认 helper 退出，再注销；保留配置、数据库、录音、模型和产物。仅当不再需要命令时删除自己的 `~/.local/bin/omm` 及 `DATA/omm/program`，**不要递归删除 DATA/omm**。模型可逐个 `omm model remove <id> --confirm`；在用模型拒绝删除。没有一键清空会议命令。

- `interaction_required` / 麦克风失败 / capture IPC 超时：原生授权界面检查；auto 关闭，先完成其它独立测试。超时本身不能证明是哪一项权限被拒绝。
- `configuration is incomplete`：按路径/模型配置逐字段补齐；认证另由 Pi 配置。
- `runtime rejected`：检查显式 `XDG_RUNTIME_DIR` 绝对 canonical 路径、本人所有/0700/本地文件系统。不要 chmod 别人的目录或改用 symlink `/tmp`。未设时 macOS 使用系统用户临时根，warning 是最佳努力生命周期说明。
- `service unavailable`：检查 `service start/status`、安装 profile 与当前 XDG 是否一致；服务注册成功不保证已运行。
- 下载/hash错误：保留已成功安装部分，检查网络/磁盘；不接受新 hash、不关闭 TLS 或校验。
- native 加载失败：检查固定版本、arm64、依赖路径/最低系统版本及包 hash，不能使用测试 C 夹具冒充原生引擎。
- Pi失败：检查专用目录认证、同一个 provider/model、可用额度与 Node；只重试纪要，不自动改模型或再转写。普通诊断不输出原始 stderr。
- `task wait` 超时：退出5，不取消持久任务；查看 `task show`。blocked/failed 用 `omm process --task <id> --retry`，不要删音频。

## 7. 桌面状态通知

通知默认启用，显示会议标识和处理状态：可信标题（最多60字，过滤控制字符）加开始日期时间、已知时长；通用或空标题回退到开始时间。失败仅附安全原因分类和实际重试状态，不发送参会者、逐字稿、路径、原始错误或凭据，也不显示任务ID。`notifications.show_meeting_title false` 隐去后续通知标题，仅显示时间/时长；遵循系统锁屏设置，AppleScript无法逐条承诺锁屏隐藏。转写单阶段处理不会误报纪要已生成；音频轮换/重连不重复弹开始/停止。

```sh
omm notifications check --json
omm notifications history --limit 20              # 人类可读会议标识/结果，无任务ID
omm notifications history --meeting <meeting-id> --json  # 精确关联；只读，不发送通知
omm config set notifications.show_meeting_title false
omm notifications request          # 仅打开系统通知设置，需用户自行操作
omm notifications test --json      # 显式发送一条无敏感内容测试，忽略事件开关
omm config set notifications.enabled false
omm config set notifications.enabled true
omm config set notifications.events '["recording_started","recording_stopped","summary_ready","recording_failed","asr_failed","summary_failed"]'
```

此版本使用系统 `/usr/bin/osascript` 的 `display notification`，固定脚本随 Rust CLI/service 编译，无额外 helper、个人脚本、扩展或安装组件。通知归属于 **macOS 的 AppleScript 执行宿主**，不是独立的 OMMCapture 或 OMM 通知应用；请以系统实际显示的宿主条目管理，不能承诺所有系统版本都出现同名权限项。`request` 只打开通知设置，不主动授权或模拟点击；`check` 的权限/可见性始终 `unknown`。调用成功不证明用户看到了横幅，系统授权、勿扰、登录桌面及通知样式都会影响显示。后台 LaunchAgent 递送仍需在用户桌面实际验证。

队列最多16条、单次调用最多3秒；调用拒绝或超时只记录脱敏 `notificationDiagnostic`（`service status`），不改变录音封存/任务结果。开始/停止按会话、纪要完成按 task/attempt 持久去重。失败不写永久 receipt：按错误阶段跨任务共享60秒冷却，使用持久化的 UTC 毫秒时间，重启不清冷却；实际尝试调用（包括拒绝/超时）才占用冷却，被跳过的错误不延长冷却，同一 task/attempt 在冷却结束后再失败可再次提醒。时间回拨重建冷却，不永久压制；业务 attempt 和产物不变，成功仍可独立提醒。

未知会议的session/开始时间为空，显示“录音启动失败／会议时间未知”；通知发生时间另列，不冒充会议开始。旧WAV只有封存时间与时长时，开始时间标为estimated并显示“约”；优先使用活动录音真实开始时间，其次读取数据库会话DATA目录内已封存的 `meeting.json`；不依赖通知缓存。该文件限256KiB，核验私有权限、规范路径、无链接打开、会话/目录及封存时间；缺失或不安全则保留估算/未知和脱敏诊断，不影响处理，也不改写历史事件。通知历史保存精确会议/任务/阶段/租约关联、时间、安全原因和实际调度后的重试次数与next_at；daemon的parent/summary失败先按原最多3次策略调度，再冻结失败lease快照，段任务沿用内部独立策略；手工process不拥有自动循环，未安排的重试显示pending。被冷却跳过的事件仍记录 `throttled`。`sent` 只表示调用成功，`visibility:unknown` 表示是否实际可见未知；另有 `failed`、`timed_out`、`disabled`、`deduplicated`。历史/格式/发送失败均不改变业务结果。队列满的提示仅在服务 stderr。停机给队列3秒处理预算后取消，并等待自有子进程确认退出；子进程回收超过3秒会记录脱敏等待提示，继续保留所有权。因此**服务总退出时间没有严格3秒上限**。通知是尽力而为，不是可靠事件订阅：崩溃、队列满、退出或系统拒绝可能漏显示，receipt 只代表调用尝试；状态和产物仍以 CLI/数据库为准。旧配置缺少 notifications 字段按默认启用解析；升级不改变已有自动录音和录音同意配置。

源码升级仍先独立审查，再按第1节部署新 CLI；无需替换录音 bundle 来增加此通知能力。未审查构建不能覆盖运行中的用户安装。如果已有录音或转写任务，先只读检查 `service status` / `task list`，不要为了验证通知中断真实处理；待安全维护时点再正常封存并停止服务，保留 config、DATA、STATE、已授权 auto/consent 和 Pi 专用认证，不把历史 auto=false 写回。只备份需替换的非秘密安装元数据。

桌面验收应在部署审查通过后只发送一条固定测试消息：可用同一用户 GUI 域的临时单次 LaunchAgent 调用 `omm notifications test --json`，验证后台运行而非只验证终端。记录调用结果与可见性证据为两个结论；只读取/截取标题为 Open Meeting Memo、正文为固定 OMM 测试通知的那张通知卡片，不抓取整个桌面或其他通知。若无法取得该卡片或系统递送证据，标记可见性未验证；设置交互由用户完成，禁止脚本点击授权。测试 LaunchAgent 不 KeepAlive，验证后移除，仅使用审查通过的用户 CLI。此机制的真实后台显示及 macOS 13 全链兼容尚未验收。

### 转写重试与升级保真

语音块检查点位于受保护的 `CACHE/omm/<task-id>/.chunks`，与每次租约的工作目录分离。失败、超时、阻塞和排队后的维护/重启仅清理已退出的租约工作目录，保留已提交块；活动租约、音频读者或尚未确认退出的自有worker保护清理。全部任务完成后才可自动回收其检查点。不要手动删除未完成任务的 `.chunks`；缓存是可恢复的优化，不替代原音频和数据库中的最终产物。

新版分块版本2把原音频hash、实际预处理输入hash、模型/ASR配置及声源绑定到检查点。旧版本块保守保留但不作为新版命中，首次新版尝试可能重做这些旧块；含padding的实际模型输入最长25秒。部署前正常停止服务并等待自有子进程退出，保留旧二进制与非秘密安装元数据、DATA/STATE/CONFIG及缓存；无需手改任务数据库。回滚同样先停止再恢复旧二进制，旧录音不改写。旧整会WAV的 `task show` 也返回无正文的 `stageProgress`（声源、完成块数、语音样本数、处理时长与UTC观察时间）和带UTC首次观察时间的 `stageErrors`；旧错误记录缺时间时保持未知，不能猜成准确发生时间。

## Pi工作目录留存

OMM只保证已登记、持有句柄的Pi进程真实退出；任意启动脚本的未知后台不盲杀，也不报告全部停止。尝试启动Pi后，正常完成、取消、超时及恢复均保留 `CACHE/omm/pi/<UUID>` 私有工作目录，所有权记录标明 `background_unverified`，日志提示后台未验证。旧记录缺少标志也保守保留。目录可能含文本并持续占用空间；取消不是擦除文本，卸载仍保留数据。不要在后台活动尚未核实时手工删除这些目录。正常成品仍提交，取消不提交新纪要、不自动重新生成；录音及已有成品不删除。
