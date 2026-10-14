# Open meeting memo

Open meeting memo (OMM) 定位是开源 AI Native 的会议录音/转写/纪要 软件.

## 功能/特性

* 自动感知会议开始/结束, 自动启动/停止录音, 自动获取会议信息；会议仍是一个完整实体，默认自动按最长30分钟分段，每段封存立即本地转写，会后聚合完整转写并生成一次会议纪要。
* 最终录音采用48kHz单声道 Ogg/Opus；内部保留原WAV采集/恢复内核，以后台编码归档。用静态线程/并发限制，在整体CPU大致低于整机50%的目标内尽快处理，不引入动态控制算法。
* 以服务的形式运行在后台, 提供 MCP / CLI 供AI Agent 访问; 开放本地HTTP端口, 使用浏览器访问本地WebUI.
* 录音文件/转写稿/会议纪要全程保存在本地.
* 支持OS: MacOS/Windows/Linux(Ubuntu).
* 支持的会议软件: Lark/飞书/zoom/腾讯会议/钉钉会议.
* 支持的模型接口: OpenAI 兼容接口; 支持调用本地Agent CLI: Claude code/Pi Agent/ codex / OpenCode.
* 无需管理员/root/admin权限.

## 设计要求

* 设计上充分使用插件/adapter设计模式, 第一版只支持: MacOS / Lark meeting / Pi Agent(调用大模型) / cli (mcp 和 webui 先放一放), 但架构上要方便后续扩展;
* 实现上参考[../Yulu](../Yulu/), 包括: 录音, 本地语音转写文字, 检测lark会议启停获取会议信息. 特别是录音部分的实现, 务必照抄. 很多其它软件使用其他录音方式(安装虚拟驱动之类的), 经常有兼容性问题.
