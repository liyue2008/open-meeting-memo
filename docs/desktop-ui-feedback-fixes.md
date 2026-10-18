# 桌面界面：Mantine 迁移与既有修复

前端现在使用精确锁定的 `@mantine/core` / `@mantine/hooks` **9.7.3**。使用官方 `DEFAULT_THEME`，没有 `createTheme`、自建色板、字体下载或第二套控件库。原来的全局控件皮肤 `demo.css` 已删除；`styles.css` 只管理布局、尺寸、滚动、阅读断行和独立 Logo 品牌资产。React、TypeScript、Tauri 和业务接口版本不变。

## 选择与实现依据

选择采用 Main 已核验的 [9.7.3 正式发布](https://github.com/mantinedev/mantine/releases/tag/9.7.3)、React 19.2+ peer 条件、MIT 许可及社区采用情况。当前 React 19.3.1 满足条件。实施核对了官方 [入门](https://mantine.dev/getting-started/)、[Provider](https://mantine.dev/theming/mantine-provider/)、[颜色模式](https://mantine.dev/theming/color-schemes/)、[浏览器支持](https://mantine.dev/browser-support/)、[Modal](https://mantine.dev/core/modal/) 和 [Portal](https://mantine.dev/core/portal/) 文档，并检查安装版本的实际源码。

生产内容安全策略没有放宽。采用官方静态 `styles.css` 和默认变量，关闭 Provider 运行时样式注入；颜色切换使用官方 `keepTransitions` 选项，避免临时插入 style 标签。导航使用 NavLink 和纯布局容器；没有使用会插入响应式 style 的 AppShell。Modal 使用官方 Portal、FocusTrap、标题/关闭与焦点恢复；滚动锁通过静态布局规则完成。仅保留“聚焦控件被禁用或移除且焦点失效”的窄恢复，避免 WebKit 丢到 BODY；不在普通渲染时抢焦点。

Light、Dark、系统 auto 均保留。旧 `omm.theme=system`、`omm.sidebar` 的键和值不变；跨窗口更新不回写，未知值不被粗暴覆盖。

## 全页面覆盖

| 页面/区域 | 官方组件与保留行为 |
|---|---|
| 导航、外观、公共反馈 | NavLink、Tooltip、Paper、NativeSelect、Alert、Notification；提示位于页面普通文档流，不覆盖保存按钮 |
| 会议库 | 搜索/日期快捷（默认 7 天）、NativeSelect、Checkbox、Table.ScrollContainer、Pagination；原游标和修订控制不变 |
| 详情 | 顶部播放器、Tabs 三标签、产物版本、Markdown 阅读；Tabs 不保留隐藏内容，单活媒体清理不变 |
| 处理队列 | 官方表格、状态 Badge、任务详情及嵌套取消确认 |
| 设置 | Fieldset、TextInput、Textarea、Switch、Accordion；五个可编辑组和只读存储/模型概览，保存、校验、草稿保护不变 |
| 状态与诊断 | Alert、状态 Badge、服务计划及两步恢复、诊断导出与纯文本人工指导 |
| 使用帮助 | Paper、Title、Text、Accordion；不增加行动项管理 |
| 弹窗 | 创建/导入、编辑、回收/恢复、重处理、任务详情/取消、自动录音确认、草稿丢弃/离页、初始化及服务启动/停止/更新登记 |

长标题、路径、错误、Switch 字段错误在所属容器收缩断行；完整内容保留。表格和长代码允许容器内部横滚，不用页面级 overflow 裁剪掩盖错误。保存区处于普通文档流。

## 必须保留的边界

- 原生 bridge、生成契约、请求去重、只读准入、配置修订检查均未修改。`autoRecord` / `recordingConsent` 为 null 时显示未知，不当作 false；unknown 不显示未录音。
- 旧服务登记采用和启动分开。阻断原因、纯文本指导、失败/过期后的显式重检、忙态禁用保持。启动仍按已保存偏好运行，不承诺绝不录音。
- 单次明确播放、迟到授权/播放清理与 `pause → removeAttribute('src') → load` 保持。关闭授权不能撤销浏览器已缓冲字节。用户听到的回声原因尚未确认。
- Markdown 的 raw HTML、链接和内容安全规则不变。没有新增上传、认证或权限绕过。

## 依赖许可证

新增传递依赖的许可证均进入离线 `THIRD-PARTY-NOTICES.txt`。`react-remove-scroll-bar@2.3.8` 的发布 tarball 缺少全文，使用 [精确单项映射](../apps/desktop/licenses/react-remove-scroll-bar-2.3.8.notice.json) 和同官方项目固定 commit 的 MIT 原文补齐。该发布包声明的 gitHead 在公开仓库未解析，不能把此全文说成来自 2.3.8 release 树。映射校验包名、版本、完整性、MIT/repository 声明及原文 SHA-256；其它缺文继续失败。10 项正负测试覆盖该边界。

## 验证入口与证据边界

```sh
cd apps/desktop
npm test
npm run test:notices
npm run typecheck
npm run format:check
npm run validators:check
npm run build
E=$(mktemp -d /private/tmp/omm-mantine-wk.XXXXXX)
node tests/ui-layout-wkwebview/run.mjs "$E"
# 可选小范围组件检查：--smoke 或 --dialogs
```

当前组件矩阵使用真实非持久 WKWebView，覆盖实际内容 **800×568 / 1280×788**、明暗、侧栏展收、resize、五路由与详情标签的正常/空/失败/长文本/待返回查询、设置分组和长校验、服务 busy/失败/重检/过期、全部弹窗、系统模式及切换。检查边界、滚到底、按钮命中、焦点、严格响应校验和内容安全策略，并绑定复制前后源 hash 与实际 HTML/CSS/driver/host。

夹具只使用复制 React 和严格校验的合成桥。固定合成 WAV 仅做离线 metadata、暂停 seek 和离页释放；所有 play 被拦截。它不是生产 Tauri 协议、真实 archive/Range、系统剪贴板或听感验收。最终生产整合、独立评审、安装及个人服务操作由整合方单独完成。
