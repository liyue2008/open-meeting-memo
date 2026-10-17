# 桌面版本验收记录

**当前仅 Foundation 组件，不是产品验收通过。** 下列最终门禁由 integration 在冻结当前源后运行并写入准确 exitcode、源/安装 hash 和私有 `/tmp` 证据路径。不能用旧源结果、mock、编译成功或服务注册成功代替实际功能与安装验证。

| 验收项 | 当前状态 | 最终需要的证据 |
|---|---|---|
| A1 工程/可重复入口 | Foundation 建立 | 独立worktree/分支、Rust workspace＋npm双锁、清晰Swift原目录、统一脚本、clean committed source |
| A2 共享契约 | Foundation结构测试；业务pending | 33命令、schema/Rust/TS无漂移、所有共享交换；真实桥适配器、协议/错误/预算/取消/CAS严格测试 |
| A3 Rust | Foundation针对性运行；全量pending | fmt/test --locked --all-targets/clippy -Dwarnings/build；desktop workspace同等门禁；持久并发/metadata/CRUD/import/版本/取消/config/文件安全fixture |
| A4 Swift | 本组件未改；pending | `build-dev.sh`＋AGENTS两组指定synthetic IPC ignored；不运行所有ignored或硬件 |
| A5 TypeScript/界面 | Foundation类型/骨架测试；功能pending | npm ci、严格类型、DOM交互、真实桥、日期/DST/分页/脏态/冲突、Markdown安全、明暗/侧栏全部设计 |
| A6 媒体/受托窗口 | deny-default基线；pending | 最早实际WKWebView来源/driver证明；verified同FD/hash/读锁、grant失效、Range/HEAD/206/416/ETag与并发/缓冲预算fixture |
| A7 全功能WKWebView | pending | 私有profile＋真实new core/bridge＋fake helper/worker/Pi/notifier；实际窗口/交互/搜索/CRUD/版本/任务/配置/导出/明暗/侧栏；静音metadata/解码/暂停态段内跨段seek，禁止play |
| A8 release包 | pending；Foundation禁止发布空壳 | 锁定aarch64 release app/dmg；macOS≥15.5、CLI/worker/helper/sherpa/codec资源闭包、嵌套身份/entitlements、内外ad-hoc签名、MIT/LGPL等完整许可与源码义务 |
| A9 实际安装/启动 | pending，未操作个人profile | metadata-only预检、非秘密元数据备份、用户目录真实生产包、保留录音/配置/认证/auto/consent；私有profile实际启动、窗口/正常桥验证、构建/安装来源hash及版本一致 |
| A10 独立评审 | pending | fresh只读正式评审无阻塞；修复后在最终当前源重跑所需门禁 |
| A11 SSH/PR | pending；当前禁止push | 最终clean commit经SSH推origin正式分支；远端full HEAD、PR API head、安装来源与源版本一致；创建PR不合并 |
| 跨版本/offline写权 | 契约冻结；pending实现 | 旧daemon能力缺失、旧DB只读无迁移、全mutation提交前锁内generation重验、旧配置byte不变、offline双进程/跨profile重放/过期/旧daemon竞态拒写；显式切换诊断准确 |

## Foundation 自身检查

交接报告（位于本次任务的私有工件目录）记录组件命令的真实 exitcode 与日志；不把基础编译/fixture结论升级为 A3–A11 完成。

```sh
bash scripts/test-desktop.sh --foundation
cargo clippy --locked --workspace --all-targets -- -D warnings
bash scripts/build-desktop.sh
node scripts/generate-desktop-contracts.mjs --check
git diff --check
git diff --cached --name-only # 交接提交后必须为空
```

静态Demo/Logo导入须逐字节匹配批准的13文件hash，原设计树保持不变。README只局部增加Logo并准确链接桌面状态。根Cargo.lock保留所有原package版本；新依赖只为Tauri必要闭包。生产目录无demo/mock逻辑、无已授权命令、无任意asset路径协议。

## 测试安全边界

不播放任何音频，不启动录音硬件，不发真实通知，不读个人会议/认证，不调用真实云模型，不下载权重。使用0700私有临时根、合成静音WAV、fake进程和受控C API。截图/长日志/生成bundle不入git。fixture不能证明系统真实授权、会议检测准确度、语音识别质量、个人Pi认证、可听播放或长会议质量。ad-hoc开发签名不能称为公证。
