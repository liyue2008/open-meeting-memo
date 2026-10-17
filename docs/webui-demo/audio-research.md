# macOS 桌面回听：官方文档核对

2026-10-17，仅阅读官方文档与固定版本源码。**本轮没有创建 Tauri 工程、调用媒体 API、读取录音或播放声音；静态 Demo 仍是无声时间轴。** 用户已批准 Tauri 2 桌面方向，正式实现须等新版 Demo 确认。

## 已证实的格式边界

| 项目 | 官方结论 | 对 OMM 的含义 |
| --- | --- | --- |
| Ogg 容器 + Opus 编码 | [WebKit for Safari 18.4](https://webkit.org/blog/16574/webkit-features-in-safari-18-4/#media) 明确在 **macOS Sequoia 15.4**、iOS/iPadOS 18.4、visionOS 2.4 加入 Ogg 容器的 Opus/Vorbis 支持。 | 不能再笼统说「macOS WebKit 不支持 Ogg/Opus」。现有公开混音正是 Ogg/Opus，不是 WebM/Opus 或 MP4/Opus。较早版本支持 Opus 编码，不会自动使 Ogg 容器受支持。 |
| Safari 与桌面 WebView | [Tauri 2 WebView 文档](https://v2.tauri.app/reference/webview-versions/)：macOS 使用系统 WKWebView，其 WebKit 随系统更新；与独立 Safari 版本有关联但不能只按浏览器版本推断。WebKit 18.4 公告列 Safari 同时可安装于 Sonoma/Ventura，却将上述 Ogg 支持明确限于 Sequoia 15.4。 | 目标环境需要记录 macOS 与系统 WebKit 版本；不能用「已装 Safari 18.4」代替确认。15.4 是公告给出的新增支持边界，不将所有较老系统一概判为不可播，也不承诺兼容它们。 |
| PCM WAV | Apple [Core Audio 的 OS X 支持格式表](https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/CoreAudioOverview/SupportedAudioFormatsMacOSX/SupportedAudioFormatsMacOSX.html)列 WAVE 中的 LEUI8、LEI16/24/32、LEF32/64 等 PCM；[Safari HTML5 音视频历史指南](https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/Using_HTML5_Audio_Video/AudioandVideoTagBasics/AudioandVideoTagBasics.html)列 iOS WAVE，桌面则依当时 QuickTime 能力。 | WAV 是容器，PCM 是编码，不能把所有 WAV 都视为同一格式。上述归档文档说明原生格式基础，**不是当前 WKWebView 对每个位深、采样率、路径协议与 seek 的完整保证**。优先验证明确参数的 PCM WAV（如16-bit little-endian），再决定兼容缓存。 |

`canPlayType('audio/ogg; codecs="opus"')` / `canPlayType('audio/wav')` 只能初筛；实际解码、时长、Range 和跳转仍需合成无声验证。MediaRecorder 可生成 PCM 的说明讲的是录制输出，不能拿它独自证明 WAV 回听。

## Tauri 媒体通路：已有基础不等于完整方案

- [convertFileSrc](https://v2.tauri.app/reference/javascript/api/namespacecore/#convertfilesrc) 只将路径转为资产 URL；使用还需 `assetProtocol` 的 scope 与 CSP。不能据此开放整个 DATA，也不能认为生成 URL 就完成安全串流。
- [Tauri 2.12.4 的官方 asset 实现](https://github.com/tauri-apps/tauri/blob/tauri-v2.12.4/crates/tauri/src/protocol/asset.rs) 已解析 Range、生成部分响应/Content-Range，并处理不可满足的范围；这是固定源码核对，不是本轮集成验证或对未来版本的承诺。该通路不自动提供 OMM 的产物 hash、已提交状态、读者锁或跨段整场时间轴。
- [自定义协议接口](https://docs.rs/tauri/2.12.4/tauri/struct.Builder.html#method.register_asynchronous_uri_scheme_protocol) 可以返回自定义响应；是否走该方案，在正式设计中验证。建议窄协议仅接不透明会议/段标识，核心查已提交公开 mix、受控根/所有权/symlink/类型/hash 后持有已验证句柄；Range/取消时继续遵守旧读者锁与 cleanup 互斥，不能绕过现有安全检查。
- byte Range 仅对应某个文件，**不把多段 Ogg 直接拼成单个文件**。播放器以段偏移映射整场时间，跨段 seek 装载目标段；缺口/未完成归档明确显示。文本与业务走 commands，整场音频不经 invoke 转 base64。

## 实现前的无声验证门槛（尚未执行）

1. 获准后用自产合成无声 Ogg/Opus 和明确参数 PCM WAV，在目标 macOS WKWebView 验证 metadata、实际解码、段内/跨段 seek、倍速与取消；不改 OS、不播放可听声音。开发所需搭建仍须等 Demo 确认。
2. 验证协议的 206/Content-Range、416、条件读取与资源边界；读者期间 cleanup、替换/hash 改变、恶意路径、并发及断开释放均有合成 fixture。浏览器 Chrome 静态测试不能代替这一步。
3. 仅当目标解码确有缺口，再确认从**已校验公开混音**生成有界 PCM WAV 缓存的范围/成本；不恢复内部 mic/system、不改变归档或转写引擎、不临时引入随机编码器。现有 service 仍是唯一录音及任务 owner，Swift 采集内核/身份不变。

实际可听播放、真实录音权限及打包身份验收分别取得授权；阅读文档不会使这些验证自动完成。
