import { Alert, Button, Paper, Text, Title } from '@mantine/core';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { compatibilityReasonText, useAction, useBridge, useInfo } from '../app/context';
import { dateTime } from '../app/dates';
import { ErrorNotice, Heading, Loading } from '../components/ui';
import { ServiceLifecycle } from './lifecycle';
const compatibilityLabel = {
  compatible: '兼容',
  read_only: '只读',
  requires_switch: '需要安全切换',
  unavailable: '不可用',
  unknown: '未知',
};
const modeLabel = { none: '不可写', service: '后台服务授权', offline: '已验证的离线资料库' };
const notificationLabel = {
  recording_started: '录音开始',
  recording_stopped: '录音停止',
  summary_ready: '纪要完成',
  recording_failed: '录音失败',
  asr_failed: '转写失败',
  summary_failed: '纪要失败',
  test: '显式测试',
};
const deliveryLabel = {
  sent: '系统调用返回',
  failed: '调用失败',
  timed_out: '调用超时',
  disabled: '已禁用',
  deduplicated: '重复事件未调用',
  throttled: '频率限制未调用',
};
const stateLabel = { ready: '可用', unavailable: '不可用', blocked: '被阻止', unknown: '未知' };
export function DiagnosticsPage() {
  const bridge = useBridge();
  const info = useInfo();
  const action = useAction();
  const diagnostics = useQuery({
    queryKey: ['diagnostics_get'],
    queryFn: ({ signal }) => bridge.call('diagnostics_get', {}, signal),
    refetchInterval: 10000,
  });
  const history = useInfiniteQuery({
    queryKey: ['notification_history'],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) =>
      bridge.call(
        'notification_history',
        { meetingId: null, cursor: pageParam, pageSize: 25 },
        signal,
      ),
    getNextPageParam: (p) => p.nextCursor ?? undefined,
  });
  const compatibility = diagnostics.data?.compatibility ?? info.data?.compatibility;
  return (
    <>
      <Heading
        title="状态与诊断"
        text="仅展示脱敏状态，不读取会议内容或认证信息；检查不会请求权限、录音或调用模型。"
      >
        <Button onClick={() => void diagnostics.refetch()} type="button" variant="default">
          刷新诊断
        </Button>
        <Button
          disabled={action.busy}
          onClick={() =>
            void action.run(async () => {
              const result = await bridge.call('diagnostics_export', {});
              return result.outcome === 'saved' ? '脱敏诊断已导出' : '已取消导出';
            })
          }
          type="button"
          variant="default"
        >
          导出脱敏诊断
        </Button>
      </Heading>
      <ServiceLifecycle />
      <Paper className="panel section-spaced" withBorder p="md" component="section">
        <Title order={2}>桌面与后台服务</Title>
        {info.error && <ErrorNotice error={info.error} retry={() => void info.refetch()} />}
        <dl>
          <dt>App 版本</dt>
          <dd>{info.data?.appVersion ?? '未知'}</dd>
          <dt>源码版本</dt>
          <dd>{info.data?.sourceRevision ?? '未知'}</dd>
          <dt>运行平台</dt>
          <dd>{info.data?.platform ?? '未知'} · macOS 15.5+</dd>
          <dt>接口版本</dt>
          <dd>{info.data?.contractVersion ?? '未知'}</dd>
          <dt>服务兼容性</dt>
          <dd>{compatibility ? compatibilityLabel[compatibility.state] : '未知'}</dd>
          <dt>写入模式</dt>
          <dd>{compatibility ? modeLabel[compatibility.admissionMode] : '未知'}</dd>
        </dl>
        {compatibility && !compatibility.writesAllowed && (
          <Alert className="notice" variant="light" role="status">
            当前只读：{compatibilityReasonText(compatibility.reason)}
            。保留既有资料；请按下方诊断建议检查服务版本。不会自动迁移或重启旧服务。
          </Alert>
        )}
        {compatibility?.safeSwitchRequired && (
          <Text component="p">
            需要显式安全切换服务版本。使用上方“安全切换服务版本”检查实际可执行计划；活动录音、处理任务或归属不明时拒绝切换，保留个人工作。
          </Text>
        )}
      </Paper>
      <Paper className="panel section-spaced" withBorder p="md" component="section">
        <Title order={2}>依赖与权限</Title>
        {diagnostics.isPending ? (
          <Loading />
        ) : diagnostics.error ? (
          <ErrorNotice error={diagnostics.error} retry={() => void diagnostics.refetch()} />
        ) : (
          <>
            {diagnostics.data.items.map((item) => (
              <article className="diagnostic-row" key={item.key}>
                <div>
                  <Title order={3}>{item.key}</Title>
                  <Text component="p">{item.message}</Text>
                  {item.action && (
                    <Text className="muted" component="p" c="dimmed">
                      建议：{item.action}
                    </Text>
                  )}
                </div>
                <span className={`badge ${item.state === 'ready' ? 'success' : 'warning'}`}>
                  {stateLabel[item.state]}
                </span>
              </article>
            ))}
            <Text className="muted" component="p" c="dimmed">
              更新时间：{dateTime(diagnostics.data.generatedAtMs)}。未知权限不能视为已授权。
            </Text>
          </>
        )}
      </Paper>
      <Paper className="panel section-spaced" withBorder p="md" component="section">
        <Title order={2}>通知调用记录</Title>
        <Text className="muted" component="p" c="dimmed">
          调用成功只表示系统脚本返回；通知是否显示和系统授权状态仍然未知。
        </Text>
        {history.isPending && <Loading />}
        {history.error && (
          <ErrorNotice error={history.error} retry={() => void history.refetch()} />
        )}{' '}
        {history.data?.pages
          .flatMap((p) => p.items)
          .map((item) => (
            <Text key={item.id} component="p">
              {dateTime(item.occurredAtMs)} · {notificationLabel[item.event]} ·{' '}
              {deliveryLabel[item.status]} · 可见性未知
              {item.safeReason && ` · ${item.safeReason}`}
            </Text>
          ))}
        {history.data?.pages.every((p) => p.items.length === 0) && (
          <Text className="empty-state" component="p">
            没有通知调用记录。
          </Text>
        )}
        {history.hasNextPage && (
          <Button
            disabled={history.isFetchingNextPage}
            onClick={() => void history.fetchNextPage()}
            type="button"
            variant="default"
          >
            更多记录
          </Button>
        )}
      </Paper>
    </>
  );
}
export function Help() {
  return (
    <>
      <Heading
        title="使用帮助"
        text="Open Meeting Memo · 本机录音、本机转写、通过专用 Pi 环境生成纪要。"
      />
      <div className="help-grid">
        <Paper className="panel" withBorder p="md" component="section">
          <Title order={2}>开始一场会议</Title>
          <ol>
            <li>
              首次使用进入“状态与诊断”，明确初始化空资料库，检查依赖和配置，再确认启动后台服务。
            </li>
            <li>核对录音权限、本地模型和 Pi；App 不会自动下载、请求权限或开始录音。</li>
            <li>在“会议库”新建会议，选择已封存音频或确认同意后手动录音。</li>
            <li>飞书自动录音只接受明确参会信号；未知状态不会当成会议结束。</li>
            <li>停止录音后等待封存；转写与纪要会在后台继续。</li>
          </ol>
          <Title order={2}>查阅与恢复</Title>
          <Text component="p">
            默认列出最近 7 个本地日历日。本周从周一开始。内容索引未完成时搜索结果可能不完整。
          </Text>
          <Text component="p">
            会议详情顶部回听已验证的公开混音。点击逐字稿时间定位；跨段保持暂停，不自动播放。旧纪要和逐字稿可以在“处理版本”中读取。
          </Text>
          <Text component="p">
            移入回收站不会删除录音、产物或任务历史。活动和未封存会议不能移除；可随时恢复。
          </Text>
          <Text component="p">行动项只读，来自纪要文本，不管理完成状态。</Text>
        </Paper>
        <Paper className="panel" withBorder p="md" component="section">
          <Title order={2}>数据与隐私</Title>
          <Text component="p">会议内容不进入浏览器本地存储。这里只保存主题和侧栏布局偏好。</Text>
          <Text component="p">
            会议 Markdown 不执行
            HTML、脚本或远程图片，也不打开任意链接。复制和导出由原生端限定到会议产物。
          </Text>
          <Text component="p">
            本地转写不上传音频。Pi 是调用所选文本模型的独立命令行程序。配置云端 Pi
            模型时会发送逐字稿文本；Pi 认证与个人工具相互隔离。
          </Text>
          <Title order={2}>命令行配套</Title>
          <Text component="p">可在终端运行以下只读入口查看当前安装支持的确切命令：</Text>
          <pre>
            omm --help
            <br />
            omm record --help
            <br />
            omm task --help
            <br />
            omm config --help
            <br />
            omm doctor
          </pre>
          <Text component="p">
            CLI 和 App
            共享同一份数据及写入保护。遇到版本冲突先保留草稿并重新读取，不要直接修改数据库。
          </Text>
          <Title order={2}>常见异常</Title>
          <Text component="p">
            旧服务不兼容：保留只读资料，按项目安装文档执行显式安全切换。模型、Pi 或 Node
            缺失：按诊断建议修复依赖后重试失败阶段。关闭窗口不会取消后台处理。
          </Text>
        </Paper>
      </div>
    </>
  );
}
