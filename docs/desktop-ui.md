# 桌面前端

生产入口是 `apps/desktop/src/main.tsx` → React `App`，仅通过
[`src/bridge/client.ts`](../apps/desktop/src/bridge/client.ts) 调用冻结的
[desktop/v1](../contracts/desktop/v1/README.md) 原生命令。打开 App 不自动初始化、启动服务、录音或调用模型；显式操作交由原生验证并执行，不在浏览器执行子进程；
Vite 开发服务器仅在显式开发命令中监听 `127.0.0.1:1420`。

## 目录与可重现命令

| 路径 | 职责 |
|---|---|
| `src/app/` | 本地日历区间、主题/侧栏偏好、查询和操作上下文 |
| `src/bridge/client.ts` | 唯一 `invoke` 边界、版本/响应标识/大小校验、有界并发、媒体地址限制 |
| `src/bridge/validators.generated.*` | 从冻结 schema 生成的独立校验器；生产不使用 `eval` 或 `Function` |
| `src/features/` | 会议库、CRUD、详情/产物/播放器、任务、配置、诊断和帮助 |
| `src/components/` | 统一图标、双色 Logo、原生模态框和中文状态 |
| `src/styles/` | 原批准 Demo 的原字节配色及样式；React 适配位于 `src/styles.css` |
| `public/` | 自包含 Logo、原 Radix Colors 许可证及完整生产 npm 依赖许可文本；不含会议内容或音频 |
| `tests/` | 合成传输、DOM 交互、日期/安全/媒体/配置测试；不进入生产依赖图 |
| `scripts/` | 构建期编译 JSON schema、收集生产依赖许可，并支持漂移检查 |

使用项目已固定的 Node/npm 版本（见 `package.json`），在仓库根执行：

```sh
cd apps/desktop
npm ci
npm run format:check
npm run typecheck
npm run contracts:check
npm run notices:check
npm test
npm run build
```

共享 schema 或 `src/contracts.generated.ts` 由单独 owner 维护，不能为绕过测试单侧修改。
共享 schema 获准更新后，运行 `npm run validators:generate`，审核生成 diff，并重新运行上述门禁。
`npm run notices:generate` 从 npm 锁和安装的 115 个生产依赖收集完整原文许可，`notices:check` 防止遗漏；该离线 notice 文件自动进入发布资源。
生产构建目标是 Safari 18.5 对应的 WebKit（macOS 最低 15.5），不以 Chromium 测试替代 WKWebView。
当前生产 JS 单块约 1.1 MB（gzip 约 195 KB），其中包含独立 schema 校验器；Vite 会提示体积警告。

## 业务行为

- **首次设置与服务**：采用共享生命周期契约提交 `a863810989500d1e61fc1fe6921f3141dd6fbd06`。
  启动仅以 `service_plan(start)` 观察真实 profile/service 状态；空资料库显示首次设置入口，明确确认后才调用 `profile_initialize({})`。
  原生确认目录为空后初始化，UI 重新读取配置；自动录音和录音同意仍关闭，服务启动另行确认。
  “状态与诊断”提供启动、停止、安全切换的实际计划、版本、状态、短期有效期和中文拒绝原因；确认后仅提交 opaque plan token。
  执行前由原生重新核对身份、配置、目标程序和忙态；错误/过期的计划不重放，必须重新获取并再次确认。
  活动录音/处理任务、未知归属、已有目录、缺包和旧服务均保留原状态并明确解释；停止/切换不会删除资料或改同意。
  生命周期操作后重新读取配置、兼容性和诊断；服务运行不冒充模型、Pi 或权限全部就绪。配置脏态导航守卫也覆盖首次设置入口。
- **会议库**：默认最近 7 个本地日历日（含今天）；今日、周一起的本周、最近 3/7 天、全部和自定义。
  发送本地日期、IANA 时区和精确 UTC 边界；跨夏令时按日历计算。标题/已提交内容搜索、来源/状态/回收站筛选、
  时间/时长排序由原生端执行。分页 25/50/100 条，以 revision 绑定 opaque cursor；快照过期返回第一页并明确提示。
  旧查询迟到不会替换新搜索结果。搜索索引未完成、总数未知和空结果均真实显示。
- **创建与保留式删除**：新建只能显式录音或选择音频导入，不创建空会议。录音须确认参与者同意；自动录音另行确认。
  停止后显示实际封存状态，退出 App 不调用停止服务。文件选择返回不含路径的 opaque handle，取消/卸载释放 handle。
  导入提交显式标题和用户填写的原会议开始时间；拒绝被系统规范化的非法日期和夏令时跳过的本地时间。标题/备注/标签使用 revision 更新；回收站与恢复全成全拒，永不永久清理。
  忙、无权、不兼容和版本冲突保留现有内容及编辑草稿。
- **详情**：顶部真实分段 `<audio>`，仅把受限 `omm-media://localhost/<grant>` 赋给音频。
  媒体 URL 必须与返回的 grant ID、所请求 revision 一致。原生控件负责用户显式播放/暂停，前端不调用 `play()`。
  时间轴、±15 秒、倍率和跨段定位使用真实 offset/duration；空隙不压缩，跨段保持暂停。
  可重新申请失效 grant，离开详情关闭 grant。无可验证公开混音时不伪造录音。
- **逐字稿/纪要**：按原生产物分块读，部分/当前/历史有明确标记；单窗口最多 32 个内容块，更多内容可原生导出。
  逐字稿本页搜索和合法时间定位、完整产物复制/导出、真实版本/哈希/血缘均使用对应命令。
  最新失败不会替换旧有效产物；可选择历史成功逐字稿作为仅重生成纪要的输入。
  行动项只从明确的纪要章节提取并只读，不包含跟进复选框。没有结构化行动项时如实说明。
- **任务**：完整任务列表、筛选、分段进度、详情、有限重试、重新入队和持久取消。
  取消明确告知需等待已拥有的子进程退出，不把请求受理说成已经取消。重处理保留旧产物，批量结果逐项显示。
- **设置**：冻结 `EditableConfig` 的全部字段，严格数字解析、原生校验、生效时机、读取版本与原子保存。
  保存冲突不覆盖磁盘，保留草稿。导航/关窗提供保存、放弃、继续编辑；明确确认后才重新读取。
  未初始化的默认值明确标记且不可编辑或保存，首次设置后才获得真实配置；`autoRecord`/`recordingConsent` 不进入普通配置 patch；Pi 凭据不读取或保存。
- **状态/帮助**：读取真实依赖、权限、兼容性、来源版本和通知调用历史，原生导出脱敏诊断。
  未知权限不能称已授权；通知调用返回不能称横幅已显示。不自动安装、迁移、启动服务、请求权限或探测模型。
  旧/未知服务的一般写入被 admission 检查阻止；关键首次启用及受保护服务操作在 App 内完成，CLI 帮助作为辅助。

## 外观、可访问性和内容隔离

[批准 Demo](webui-demo/README.md) 的 `styles.css`、`palette.css`、Radix 许可证及独立 Logo 原字节保留在前端副本；
不执行 Demo 的 `app.js`、`demo-data.js`、主题或侧栏脚本。原浅色、柔和深色与青绿点缀保持不变。
默认实时跟随操作系统，也可手动覆盖；只持久保存 `omm.theme`、`omm.sidebar` 两个布局键。
损坏/拒读/拒写浏览器存储时明确降级。录音、搜索词、标题、纪要、配置或授权 token 不写 web storage。
查询缓存只在内存，窗口关闭后消失；导航只保存当前页面，不把内容放在 URL。

侧栏统一 24 单位 SVG 图标，桌面可持久收缩，收缩后的鼠标/键盘提示支持 Escape；窄屏仍为文档流导航。
支持跳到正文、页面切换焦点、中文标签、原生模态框焦点/取消、忙态禁用、错误/状态通知，内容 tabs 支持方向键、Home/End。

Markdown 使用本地 `react-markdown` 和 GFM 解析器：禁用原始 HTML，不加载任何图片，不输出外部链接，
只将合法且在会议时长内的时间标记转换为定位动作。禁用任务列表 input，代码块仅为文本。
不存在通用 shell、任意文件路径、任意剪贴板文本、远程资源、浏览器下载或测试授权入口。

## 原生交汇点与 WKWebView 私有 fixture driver

生产页面不暴露测试全局对象、调试桥、fixture URL 参数或测试按钮。`App` 可注入 `Bridge` 仅用于不被入口引用的单测；
生产始终创建 `createBridge()`。真实 WKWebView 测试必须由原生 test-only harness 创建私有 profile、
使用真实命令/core 与 fake helper/worker/Pi/notifier，再通过当前受托窗口执行 DOM 操作并回收测试结果。
不能把本目录 jsdom 的假传输称为真实桥、解码或权限证据。

给原生 owner 的稳定 DOM 定位面（中文可访问名）：

| 操作 | 定位面 |
|---|---|
| 导航/主题/侧栏 | `a[aria-label="会议库"]`、`a[aria-label="设置"]`、`select[aria-label="外观主题"]`、`button[aria-label="收起侧栏"]` |
| 搜索与列表 | `input[aria-label="搜索会议"]`、`section[aria-label="会议列表"]`、标题按钮、`下一页` / `上一页` |
| 创建/导入/录音 | `＋ 新建会议`、`dialog` 中标签 `会议标题` / `原会议开始时间（本地）`，`选择音频文件` / `导入` / `开始录音` |
| 批量/回收站 | `input[aria-label="选择本页全部会议"]`、`移入回收站` / `确认恢复`，`位置` select |
| 内容/任务 | `[role="tab"]` 的 `会议纪要` / `逐字稿` / `处理版本`；`重新处理`、`重试失败阶段`、`取消任务` |
| 首次设置/服务 | `打开首次设置` / `打开服务设置`；`section[aria-label="首次设置与后台服务"]`；`初始化本机资料库` → `确认初始化`；`启动后台服务` / `停止后台服务` / `安全切换服务版本` → 对应确认按钮；失败需 `重新检查计划` |
| 配置 | 字段的原生 label（如 `Pi 模型` / `转写线程数`）、`保存配置`、`设置尚未保存` dialog |
| 媒体 | `audio[aria-label="录音播放控件"]`、`input[aria-label="会议时间定位"]`、`select[aria-label="录音分段"]` |

受控 driver 更改 React 输入时需调用原生 HTMLInputElement value setter，再发送冒泡 `input`；select 发送 `change`。
通过 DOM 的实际状态/错误等待条件同步，不能靠固定睡眠或在 production 注入假成功。
媒体只读取 `duration`/`readyState`/`paused`，设置静音 fixture 的 `currentTime` 并等待 seeked；绝不调用 `play()`。
实际 WebKit 解码、Range 与来源身份由原生验收收集。前端要求精确本地主机媒体 URL，不能为修复测试放宽成 file/HTTP/通用路径。

设置的原生关窗守卫使用 `getCurrentWindow().onCloseRequested`，脏态保存/放弃后调用该窗口的 `destroy()`。
原生需要仅对受托主窗口允许事件 listen/unlisten 和 window destroy；不需要 remote capability、通用 window 创建或 shell。
能力范围及最终 CSP 由 native owner 实现并做真实测试，前端单测中的 mock 不证明这些权限可用。

## 前端测试证据边界

当前测试包括共享 48 个合成交换的真实适配器参数/响应验证、共享 schema 断言、UTF-8/正文预算、
四个并发请求及排队取消/请求标识、媒体 URL 和 grant/revision 绑定；DOM 交互覆盖库查询/分页/竞态、
CRUD/导入 handle/录音同意、封存/自动录音失败、任务重试/取消/入队、历史输入和只读 Markdown、
暂停态跨段 seek/grant 释放、配置校验/CAS 冲突/脏态和关窗、主题/侧栏/存储失败、键盘 tabs；日期另用独立 Node 时区进程覆盖夏令时。
首次设置与服务另覆盖无副作用打开、初始化确认/冲突/已有目录、独立启动、停止/切换、忙态/未知归属/旧版本拒绝、单次计划失败后重新确认、过期/错动作和配置脏态守卫。
测试将 `HTMLMediaElement.play()` 设置为抛错，禁止隐式播放。

这些是 UI 组件及合成传输证据。完整产品仍须整合后在真实 WKWebView 私有 fixture 下运行当前版本、
原生安全/资源闭包/release/用户目录安装及 fresh 独立评审；不能据此宣称真实权限、ASR、Pi 认证、通知可见性或长会议质量已经通过。
