# 桌面能力与验证入口

这是候选审查的源码导航，不是全部门禁已通过的声明。实际运行的提交、退出码和来源hash以[验收记录](desktop-acceptance.md)及其私有证据为准；组件通过不能替代当前组合源验证。桌面只通过冻结的36个typed命令进入共享应用层，不在React或Tauri handler复制业务规则。

| 用户能力 | CLI或共享用例 | 真实桌面入口 | 主要验证源码 |
|---|---|---|---|
| 本地今日/本周/3/7天，默认最近7天 | `meeting list --from` / `meeting_list` | 会议库日期筛选 | `apps/desktop/tests/dates.test.ts`、`scripts/fixtures/desktop-ui-probe.js` |
| 查询、状态/来源、稳定排序与分页 | `meeting list --query/--source/--status` / `meeting_list` | 会议库筛选和加载更多 | `apps/desktop/tests/pagination-races.test.tsx`、`tests/desktop_native.rs::desktop_content_search_pagination_and_artifact_versions` |
| 元数据编辑与CAS冲突 | `meeting update --expected-revision` / `meeting_update` | 详情编辑对话框 | `tests/desktop_native.rs::desktop_metadata_cas_idempotency_rollback_and_retained_trash`、`desktop_cas_concurrent_writers_have_single_winner` |
| 保留删除、回收站、恢复 | `meeting trash/restore` | 详情确认/回收站恢复 | `scripts/fixtures/desktop-ui-probe.js`、`tests/desktop_native.rs` |
| 详情、转写/纪要成功版本 | `meeting show/read` / `artifact_list/read` | 详情tab与版本选择 | `apps/desktop/tests/detail-media.test.tsx`、`tests/desktop_native.rs::desktop_cli_reads_retained_success_and_shares_metadata_and_trash` |
| 非空只读行动项及安全Markdown | 共享已验证summary artifact | 行动项列表；无交互checkbox | `scripts/fixtures/desktop-ui-probe.js`、`apps/desktop/src/features/artifacts.tsx` |
| 录音上置、受限回听、速度、时间定位 | `media_manifest/open/close` | 详情上方真实audio控件 | `apps/desktop/tests/detail-media.test.tsx`、`apps/desktop/src-tauri/src/host/media_revision.rs`、`scripts/fixtures/desktop-ui-probe.js` |
| 受限Range/HEAD/If-Range、hash与reader生命周期 | verified FD及artifact grant | 自有媒体协议，不接受任意路径 | `tests/desktop_native.rs::desktop_media_verified_fd_ranges_hash_and_symlink_safety`、`desktop_media_permission_change_is_rejected` |
| 受限导入及取消 | `meeting import` / `import_select/commit/cancel` | 原生选择器与实际导入确认 | `tests/desktop_native.rs::desktop_import_normalizes_mixed_lineage_deduplicates_and_binds_generation`、`apps/desktop/src-tauri/fixtures/bridge-probe.js` |
| 文本复制、单场及批量导出 | `meeting read` / `artifact_copy/export`、`meeting_batch_export` | 详情/会议库批操作；原生受限目标 | `scripts/fixtures/desktop-ui-probe.js`、`apps/desktop/src-tauri/fixtures/bridge-probe.js` |
| 重转写/重纪要、旧成功版本保留 | `process --stage ... --force` / `processing_enqueue` | 详情重新处理 | `tests/desktop_native.rs`、`scripts/fixtures/desktop-ui-probe.js` |
| 失败队列、重试/取消/重新入队 | `task list/show/cancel/requeue`、`process --task ... --retry` | 队列及任务详情 | `tests/desktop_native.rs::desktop_persistent_cancel_waits_for_owned_worker_before_settling`、`apps/desktop/tests/recording-recovery.test.tsx` |
| 配置校验、hash CAS、冲突保草稿 | `config show/validate/set` | 设置表单/校验/保存/重载确认 | `tests/desktop_native.rs::desktop_config_raw_hash_and_writer_lock_prevent_lost_updates`、`desktop_configuration_journal_recovers_published_target_without_reapplying` |
| 脏态导航与真实关窗 | 原React dirty guard及受限测试close入口 | 留在设置/放弃并离开 | `apps/desktop/tests/native-close.test.tsx`、`scripts/fixtures/desktop-ui-probe.js`；native观察实际Destroyed和owned收尾 |
| 首次显式初始化 | `init` / `profile_initialize` | 首次设置，独立确认 | `apps/desktop/tests/lifecycle.test.tsx`、`apps/desktop/src/features/lifecycle.tsx`；实际React生命周期组合门禁仍单列 |
| 启动/停止/安全切换后台服务 | `service start/stop/status` / `service_plan/apply` | 新鲜计划、到期拒绝、实际确认和重检 | `apps/desktop/src-tauri/src/bridge_fixture/lifecycle.rs`、`packaging/desktop/test-lifecycle.sh`；必须当前资源包及真实React按钮证据 |
| 录制与停止封存、自动录音独立同意 | `record start/stop/status/auto` | 录音控制及独立同意 | `apps/desktop/src-tauri/fixtures/bridge-probe.js`、`scripts/fixtures/desktop-ui-probe.js`；仅fake helper，无硬件 |
| 诊断、通知历史、无内容诊断导出 | `doctor`、`notifications history/check` / `diagnostics_get/export` | 状态与诊断、帮助 | `scripts/fixtures/desktop-ui-probe.js`、`apps/desktop/src-tauri/fixtures/bridge-probe.js` |
| 明暗、OS匹配、窄屏、统一图标/双色Logo | 本机UI偏好，不含会议内容 | 主题选择、侧栏、响应式布局 | `apps/desktop/tests/preferences-safety.test.tsx`、`scripts/fixtures/desktop-ui-probe.js` |
| 两独立App进程保留主题/收缩 | 测试专属UUID网站数据存储 | 第一进程真实写入、第二进程初始还原 | `scripts/fixtures/desktop-preferences-probe.js`、`scripts/test-desktop-preferences-wkwebview.sh`；先A/B隔离，最后精确owned清理 |
| 受托窗口/命令ACL/严格DTO | 冻结契约；无任意路径或shell | 仅trusted main本地窗口 | `apps/desktop/tests/desktop_contracts.test.mjs`、`production-validators.test.ts`、`apps/desktop/src-tauri/src/bridge_fixture/window_acl.rs` |
| legacy/unknown只读及安全offline写入 | 共享profile/generation/admission锁 | 打开App不自动初始化/迁移/启动 | `tests/desktop_native.rs::desktop_legacy_profile_read_does_not_migrate_or_grant_writes`、`desktop_unknown_future_owner_is_not_downgraded_on_explicit_service_start` |
| 多语言构建、资源闭包与用户包 | 根Rust workspace、React/TS、Swift helper | `scripts/{dev,test,build,package,install}-desktop.sh` | `packaging/desktop/resources.py`、`scripts/test-desktop-entrypoints.sh`；release需clean当前来源，安装另行验收 |

## 明确边界

- `permissions request`、`notifications request/test`、模型/codec安装、LaunchAgent注册/卸载及Pi/Node/Lark外部配置仍是明确的用户运维动作。App诊断和帮助说明实际状态/步骤；打开App不下载、请求权限、测试通知、改个人认证或自动安装服务。冻结36命令没有暗藏任意工具安装或shell入口。
- CLI与App复用会议/配置/任务/录制/服务应用层；不声称每个CLI运维子命令都有同名GUI按钮。系统工具、模型和认证的实际就绪必须另验。
- fixture用虚构内容、fake helper/worker/Pi/notifier。媒体只测metadata/无输出解码/暂停seek，`playCalls=0`；不证明实际播放、录音权限、ASR质量、会议检测、云认证、通知可见性或长会议质量。
- UUID测试只处理工具分配且拥有journal/nonce的两项存储；不读取或清理personal/default store，不造UUID绕准入，不改全局OS主题。同窗匹配不冒称实测OS切换。
