# 模型清单交接

`models.json` 是固定 FireRedASR INT8 + Silero VAD 清单；本模块不实现隐式下载，不启动真实/付费模型，不读取用户会议材料。URL、SHA-256 与大小采用设计所依据的只读 Yulu 公开模型配置；本次没有下载 1.74GB 权重重新计算哈希。

core 的显式模型下载命令消费：

- `schema_version=1`、`native_version=1.13.2`、`model_version` 必须与 `omm::adapters::asr::models::MODEL_VERSION` 相同。
- FireRed 包为 `.tar.bz2`，选取包根 `sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16/` 中 `encoder.int8.onnx`、`decoder.int8.onnx`、`tokens.txt`；不要启用未列入清单的浮点模型或其他引擎。
- Silero 是独立 `silero_vad.onnx` 文件。包完整校验后，落盘文件还须分别验证 `files` 中的哈希。
- `archive_bytes`、`installed_bytes` 是参考大小，用于预检/显示，不是代替哈希的真实性证据；新来源/新权重需要显式新清单/新版本，不得静默接纳。
- 安全下载/解包、磁盘预检、原子安装、符号链接/路径逃逸防护和许可说明由 core 的显式安装命令完成。按设计遵守模型各自的官方许可，不把程序自身许可证当作权重授权。

模型/库文件必须由配置快照显式定位。Rust 验证器对缺文件报 `unavailable`，对哈希不符报 `corrupt`，对不支持的配置版本报 `invalid_input`。没有下载重试、Python 回退或空转写成功。

动态库不在此模型清单中：目前只钉住官方 C 头文件和 API 版本，尚无已获取、验证的官方 macOS arm64 动态库包及哈希。正式安装/分发前须独立确认库与 ONNX Runtime 依赖的来源、架构和系统最低版本，并记录二进制/包哈希；不要以 C 夹具测试代替它。
