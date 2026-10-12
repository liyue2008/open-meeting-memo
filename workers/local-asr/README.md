# 本地转写 worker（首版组件候选）

实现 Rust 音频流水线和 sherpa-onnx **1.13.2 的原生 C API 调用**。推理实际由 C++/ONNX Runtime 完成，不是纯 Rust，也不执行或分发 Python。尚未装载官方动态库或真实权重；C 夹具成功不代表真实模型的准确率或性能验证。

## core 接入

- 静态模块 `omm::adapters::asr`（`asr` feature）；core 已提供 `omm-asr-worker` 二进制入口，调用 `worker_main()`。core 用 `LocalAsrAdapter::new(绝对可信worker路径)` 注入 `AsrAdapter`。
- 配置必须显式给出绝对 `library_path`（实际安装的 `libsherpa-onnx-c-api.dylib`）、encoder/decoder/tokens/vad 路径，以及 `models::MODEL_VERSION`、正数超时和 1–64 个 CPU 线程。未配置不能成功。没有 PATH、用户 Yulu 配置或全局库搜索回退。
- 1.13.2 动态库和它的依赖须由后续显式安装/打包流程供给；本组件不下载/安装它们。加载器校验 `SherpaOnnxGetVersionStr()=="1.13.2"` 及所有消费的符号，**版本字符串不是二进制完整性证明**。当前没有已验证的官方 native 二进制哈希，不虚构兼容性或来源。
- `packaging/asr/models.json` 提供固定模型版本、下载 URL、包/文件 SHA-256。`models::verify_models()` 只读验证，不修复或触发网络；下载只能由 core 的用户显式命令启动。core 应快照同一 `MODEL_VERSION`，并按输入/配置及租约进行最终提交。

## 子进程协议与失败

固定 argv：`["--protocol", "1"]`。stdin 是一个 `AsrWorkerRequest {schema_version:1,request:AsrRequest}` JSON，随后关闭；stdout 是共享 `AsrWorkerEvent` 的 progress/result/error JSONL，task_id/attempt/schema_version 必须匹配。没有 shell 拼接，也没有音频外发。

父进程清空继承环境、设置 `OMP_NUM_THREADS=1`，并验证退出状态、超时、事件顺序、源/时间/版本和输入 SHA-256。请求限 1MiB、JSON 行限 16MiB、stdout 总量限 64MiB、stderr 限 64KiB；超限报 `backend`（资源/协议限制），不截断后成功。非法 JSON/标识/输入 hash 是 `corrupt`，缺库/模型是 `unavailable`；进程崩溃/非零退出无可信 error 事件为可重试 `backend`，超时为 `timeout`。原生 stderr 不进入诊断。

父进程创建 0700 的独占 attempt 临时目录，即使 worker 被杀也清理该目录；worker 创建 0700 子目录和 0600 原始浮点 PCM 缓存，成功/常规错误由 RAII 清理。**整个服务自身被强杀时**来不及执行父进程清理，core 恢复/缓存回收须识别并清理废弃 work_dir；不能删除仍有有效租约的目录。本组件不写 DB、最终逐字稿或纪要。此权限收缩不是操作系统沙箱。

## 音频和原生边界

PCM16 WAV，8–192kHz；双轨 channel 0=mic、1=system，单轨=mic，不伪造说话人。每次最多读取 32768 输入帧；两轨独立 127-tap Blackman 低通（7200Hz 截止），保留跨块滤波历史和重采样相位。转换为 16kHz 本地临时 PCM，磁盘使用约每源每秒 64kB；磁盘不足报 I/O 错误，不将合法长音频判为损坏。没有一小时/段限制。

Silero 连续接收 512 样本窗口，最后不足窗口补零并 flush；阈值 .5、静音 .7s、最短语音 .25s、最长语音设置 25s、环形缓冲 60s。及时取出并释放 VAD segment；间隔 <.2s 且合并时长 ≤25s 才合并。按最多 8 段批次（批内由长到短）使用独立 stream，解码输入两端各加 .2s，输出时间仍为未 padding 的 VAD 边界、加封存段累计偏移。原生 recognizer、VAD、stream、speech、result 均有销毁保护，库寿命长于所有句柄。各批恢复时间顺序；跨源相近时间的相似回声优先保留 system，保留不同内容和同源重复。不得将 mic/system 当作具体说话人。

音频内存与会议总时长无关；批次波形/原生模型仍消耗内存，文本随内容增长且受上述协议预算约束。固定 ABI 的 VAD start 是 `int32_t`，连续单源约 37.28 小时会耗尽其样本索引，明确报 `backend`，不是新增产品时长政策。过滤沿参考保留因果延迟（<4ms），真实模型边界质量尚未测试。

## 证据、许可与验证

- [官方 1.13.2 C 头文件](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.2/sherpa-onnx/c-api/c-api.h)，完整只读副本 `include/c-api.h` 的 SHA-256 为 `437b1279047877167d8fadc74a60d47f3df514d703fdac1c1b6851da9bc2fdb4`，保留 Apache-2.0 许可 `LICENSE-sherpa-onnx`。Rust `repr(C)` 固定完整嵌套配置布局，不将 FireRed 字段截成不兼容的小配置。
- 音频处理参考只读 Yulu 的 `yulu/scripts/yulu_ui/src/pcmResampler.ts`、`yulu/scripts/yulu_ui/src/offlineTranscription.ts` 与 Python VAD/批量调用逻辑；保留原始 MIT 许可/版权 `LICENSE-Yulu`。仅移植逻辑，不分发其 Python。
- `tests/asr_layout.rs` 编译 C oracle，逐字段比较所有消费结构的 sizeof/alignment/offset；`tests/asr_fixtures/c_api_fixture.c` 使用上述官方头文件编译，断言配置、调用顺序/批量数量，注入 null/result/版本失败，检查全部句柄归零。它明确不是推理引擎。
- `tests/asr_audio.rs` 使用合成双轨、滤波跨块脉冲、抗混叠及 >1 小时稀疏 WAV；长 WAV 只验证进入有界流式读取，未完成整小时推理/性能测试。native 夹具验证跨块 VAD 合并/padding/时间戳。`tests/asr_worker.rs` 使用受控子进程验证崩溃/超时/上限、目录清理、身份与 hash；真实 worker 的缺模型/库和损坏权重错误也实际执行。
- 在当前 Darwin arm64、Rust 1.99 工具链测试；未声称 macOS 13 实机、其他架构、真实模型质量/性能、录音授权或发布已验证。最终应由 core 启用 capture+asr 完成集成，再交独立只读评审。
