# 分段转写：自产样本性能证据

## 本次选择

默认30分钟上限、48kHz单声道Ogg/Opus、48kbps可变码率；混音供用户查看，麦克风/系统声源归档供维护和重转。默认ASR请求3线程，静态预算按整机逻辑核的一半减去采集/服务和编码各一个核位，再裁剪到1–3；小机器串行共享编码位。已保存配置中的2线程保持原值。

**以前101–129秒的60秒结果使用debug worker，不能作为产品速度验收。** 以下表明确执行`target/release/omm-asr-worker`，测试父进程仍是debug不影响子worker编译档。23项exit0，worker SHA256 `efa8ed41be161437985bf14f377ecd307555d3056d185d2bf37aeb24828ab5a7`。3线程在6个有声病例中5个更快，双路20秒反而慢；4线程在公平诊断中波动更大、用CPU更多。因此不是“增加线程必然提速”。

32/48/64kbps双路20秒的文字、时间戳和角色哈希完全相同；2/3线程其余各样本结果也相同。选择48kbps作为容量与会议音质的中间值，尚无人工金标准或真实长会议准确率证据。

## Release矩阵（48kbps）

表中耗时含每次worker启动/模型校验加载及转写，不含归档编码；每次三份单路编码约2–6秒。双路会分别识别两个声源，60秒双路输入有最多120声源秒的工作量。

|自产样本|2线程秒|3线程秒|3线程整机CPU平均/峰值|
|---|---:|---:|---:|
|dual20|30.09|35.33|22.18% / 29.96%|
|mono20|17.30|15.97|19.98% / 30.02%|
|dual60|74.41|71.01|25.48% / 30.14%|
|mono60|40.03|36.93|23.18% / 30.09%|
|silence20|4.41|4.46|8.08% / 10.20%|
|noise20|4.33|4.44|8.10% / 10.24%|
|speech-noise20|26.73|24.17|22.84% / 30.14%|
|continuous20|14.71|13.38|18.45% / 30.05%|

单路持续编码与双路60秒ASR同时运行（3线程，连续12次三码归档）：77.46秒，拥有的测试父进程/ASR/编码平均34.07%、峰值40.46%；后台已有OMM平均另占16.46%，同时间点全部OMM加实验峰值59.96%。**这是两个工作实例的并发实验，不声称该机器当时全部OMM小于50%。** 正常单服务的静态预算预留采集/服务位；真实硬件采集与这些任务同时运行的完整总量仍需受控实测。没有动态CPU反馈算法，也没有关闭后台旧服务来隐藏竞争。

## 为什么比Yulu慢：同一自产60秒的公平诊断

在只读Yulu开发工作树`feat/segmented-recording`（HEAD `66f8468634b27933c23865da1e7c09b50bd3cf5a`，有未提交修改）读取`yulu/scripts/sherpa_offline_worker.py`，仅在私有target目录运行该类与已有隔离Python依赖。没有启动Yulu应用/守护进程，没有更改工作树、个人配置或认证。参考代码不是已发布成熟保证。

同一48kbps解码样本、从OMM保存完全相同的16kHz PCM、同FireRed INT8/Silero权重、sherpa-onnx 1.13.2/git、CPU provider、25秒VAD块、双方nice10；规范化文字都是567字符、hash相同。Yulu类冷启动而非常驻：2/4线程33.63/29.15秒；OMM release 2/4线程42.19/39.06秒；OMM debug2线程97.02秒，首块进度61.21秒对比release5.41秒，native处理35.75/36.67秒。确认debug Rust前处理是原基准大幅偏慢的主因，不能据此解释已安装release旧worker曾发生的长会两小时超时。

最后加分项计时：音频校验20–34ms、模型hash校验3150–3759ms、模型加载1115–1243ms、双声源PCM准备166–175ms。静音system/VAD或音频hash不是约9秒差距的主要来源。Yulu类对照不含其完整runtime-pack校验和应用前处理，不能当完整Yulu端到端时间。

两端ORT都标称1.24.4，但动态库SHA不同：OMM `3b76ed91e19443f04b79d53bf415d5fe66dd81211e3b7787ba48009e49e2d04d`，Python包`872533f130f1839a5bc01788ddb4f75c83a189763441ba1178788ed965449289`。仅target隔离副本换库诊断曾缩短native时间，但后来各档也明显波动，不能把全部差距归因于库构建；产品仍使用原官方pin，未换引擎/模型/库。C API只给整体decode调用，未虚构编码模型与解码模型各自的精确时间。

旧真实会议的两次超时原因仍未完全确认；本次没有读取、播放或外发个人录音。分段、每段/每语音块检查点避免正常失败把整场已完成识别全部重做。

## 口径、复现和证据

本机10逻辑核。每200ms采样`ps %cpu`，100%表示一个逻辑核；子进程树CPU和除以10才是整机百分比。表为父测试进程及其子进程（编码/解码/ASR），不含真实采集；另记录其它`omm`/`omm-capture`/`omm-asr-worker`，同时间点相加列全实例峰值。`ps`是采样估计，宿主未隔离、后台正在工作。

```sh
cargo build --locked --release
OMM_REAL_ASR_WORKER="$PWD/target/release/omm-asr-worker" \
OMM_REAL_ASR_MODELS="<固定已安装模型目录>" \
OMM_REAL_ASR_LIBRARY="<固定1.13.2 C API dylib绝对路径>" \
OMM_CODEC_TEST_MANIFEST="$PWD/target/audio-toolchain/manifest.json" \
OMM_BENCH_ROOT="$PWD/target/segment-benchmark-private" \
cargo test --locked --test segment_benchmark \
  segment_real_model_self_generated_20_60s_threads_bitrates -- --ignored --nocapture
```

选择器拒绝隐式debug回退；输出明示worker路径/哈希/编译档，父测试档另列。可加`OMM_BENCH_CASE=mono60`；未知样本失败。重叠用`segment_real_model_wav_baseline_and_encoder_overlap`，另须明示`OMM_BASELINE_ASR_WORKER`；本次只选择`dual60-overlap`，未执行安装服务。

证据：`target/segment-benchmark-release-20261014/results.json`（23项）；`target/segment-benchmark-release-overlap-20261014/results.json`（1项）；`target/segment-performance-fair-20261014/{metadata,results,timed-results,ort-match-results}.json`（公平诊断/分项/隔离换库）。自产转写保存在private target用于比较，本文不含正文。以上产物不入版本库；无法从这些短样本推断真实权限、硬件无断帧、长会质量或稳定提速。
