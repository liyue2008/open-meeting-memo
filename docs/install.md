# 用户级安装、依赖与故障处理

这是源码开发安装，不是签名公证的发行包。首版目标是 macOS / Apple Silicon；录音 helper 编译目标为 macOS 13+，但**本指南固定的原生库所带 ONNX Runtime 1.24.4 要求 macOS 15.5+**，不能据此宣称完整链路兼容 macOS 13。较旧系统需另行验证兼容的同版库构建；Intel 及 Windows/Linux 尚未实机验收。所有步骤用普通用户执行，无 sudo、系统服务或驱动。

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

LaunchAgent 位于 `~/Library/LaunchAgents/org.openmeetingmemo.service.plist`，不需要管理员。`KeepAlive=false`，服务停止后不会自动重启；下次登录 `RunAtLoad` 会启动。安装固定四个持久 XDG 根；更改这些根前先卸载服务，不能通过切换 shell 环境偷偷启第二套数据库。长期默认 auto 关闭，不录制个人会议；需要时显式启用：

```sh
omm config set lark_executable '<absolute trusted lark-cli>'
omm config set rtc_log_dirs '["/absolute/authorized/rtc/log/directory"]'
omm record auto enable --confirm-recording-consent
omm record auto disable
```

自动录制要求新鲜、相互匹配的会议/本人参会/本地 RTC 强信号，进程/窗口不足。不要为了检测测试查询个人会议或整段飞书聊天记录。候选确认/结束宽限可能漏录开头或延长结尾。服务正常 stop 会封存并确认 helper 退出，Pi 子进程取消后等待回收；SIGKILL 不能保证同样清理。

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
