import {
  Anchor,
  Alert,
  Badge,
  Button,
  NativeSelect,
  NavLink,
  Paper,
  Text,
  Tooltip,
} from '@mantine/core';
import { useCallback, useEffect, useRef, useState } from 'react';
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { BridgeContext, Notices, NoticeOutlet, useInfo, useBridge } from './app/context';
import { AppearanceProvider, useAppearance, type Theme } from './app/preferences';
import { createBridge, type Bridge } from './bridge/client';
import { ErrorNotice, Icon, Logo, Modal } from './components/ui';
import { Library } from './features/library';
import { Detail } from './features/detail';
import { Queue } from './features/queue';
import { Settings, type DraftGuard } from './features/settings';
import { DiagnosticsPage, Help } from './features/diagnostics';
import { RecordingBar } from './features/meeting-actions';
import { ServiceBanner } from './features/lifecycle';
const navigation = [
  { id: 'library', label: '会议库' },
  { id: 'queue', label: '处理队列' },
  { id: 'settings', label: '设置' },
  { id: 'diagnostics', label: '状态与诊断' },
  { id: 'help', label: '使用帮助' },
] as const;
type Page = (typeof navigation)[number]['id'] | 'detail';
function useSidebarCounts() {
  const bridge = useBridge();
  const meetings = useQuery({
    queryKey: ['sidebar_meeting_count'],
    queryFn: ({ signal }) =>
      bridge.call(
        'meeting_list',
        {
          filter: {
            query: '',
            scope: 'title',
            date: null,
            sources: [],
            statuses: [],
            sort: 'started_desc',
            visibility: 'active',
          },
          cursor: null,
          expectedRevision: null,
          pageSize: 1,
        },
        signal,
      ),
    refetchInterval: 10000,
  });
  const tasks = useQuery({
    queryKey: ['sidebar_task_count'],
    queryFn: ({ signal }) =>
      bridge.call(
        'task_list',
        {
          meetingId: null,
          statuses: [
            'queued',
            'transcribing',
            'transcript_committed',
            'summarizing',
            'blocked',
            'failed',
            'interrupted',
          ],
          cursor: null,
          pageSize: 100,
        },
        signal,
      ),
    refetchInterval: 10000,
  });
  return {
    library: meetings.data?.total == null || meetings.error ? '—' : String(meetings.data.total),
    queue:
      tasks.data && !tasks.error
        ? `${tasks.data.items.length}${tasks.data.nextCursor ? '+' : ''}`
        : '—',
  };
}
function Shell() {
  const counts = useSidebarCounts();
  const appearance = useAppearance();
  const info = useInfo();
  const [page, setPage] = useState<Page>('library');
  const [meeting, setMeeting] = useState('');
  const [pending, setPending] = useState<(() => void) | null>(null);
  const [saving, setSaving] = useState(false);
  const [closeError, setCloseError] = useState<unknown>(null);
  const guard = useRef<DraftGuard | null>(null);
  const main = useRef<HTMLElement>(null);
  const register = useCallback((next: DraftGuard | null) => {
    guard.current = next;
  }, []);
  const request = useCallback((action: () => void) => {
    if (guard.current?.dirty) setPending(() => action);
    else action();
  }, []);
  const go = (target: Page, id?: string) =>
    request(() => {
      if (id) setMeeting(id);
      setPage(target);
    });
  useEffect(() => {
    main.current?.focus();
  }, [page, meeting]);
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWindow()
      .onCloseRequested((event) => {
        if (guard.current?.dirty) {
          event.preventDefault();
          request(() => {
            void getCurrentWindow().destroy().catch(setCloseError);
          });
        }
      })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch(setCloseError);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [request]);
  const label =
    page === 'detail' ? '会议详情' : (navigation.find((n) => n.id === page)?.label ?? '会议库');
  return (
    <>
      <Anchor className="skip" href="#main">
        跳到主内容
      </Anchor>
      <Paper component="aside" withBorder p="md" className="sidebar" aria-label="主导航">
        <div className="sidebar-header">
          <Anchor
            className="brand"
            href="#library"
            aria-label="Open Meeting Memo 会议库"
            onClick={(e) => {
              e.preventDefault();
              go('library');
            }}
          >
            <Logo />
            <strong>
              Open Meeting Memo
              <Text component="small" size="sm" c="dimmed">
                会议记录，清晰留存
              </Text>
            </strong>
          </Anchor>
          <Button
            className="sidebar-toggle"
            aria-label={appearance.collapsed ? '展开侧栏' : '收起侧栏'}
            aria-expanded={!appearance.collapsed}
            onClick={appearance.toggleSidebar}
            type="button"
            variant="default"
          >
            <Icon name="collapse" />
            <span className="sidebar-toggle-label">收起侧栏</span>
          </Button>
        </div>
        <div className="workspace-label">
          本机工作空间 <span>LOCAL</span>
        </div>
        <nav>
          {navigation.map((n) => (
            <Tooltip
              key={n.id}
              label={n.label}
              events={{ hover: true, focus: true, touch: false }}
              openDelay={0}
              closeDelay={0}
              disabled={!appearance.collapsed}
              position="right"
              transitionProps={{ duration: 0 }}
            >
              <NavLink
                component="a"
                href={`#${n.id}`}
                aria-label={n.label}
                aria-current={
                  page === n.id || (page === 'detail' && n.id === 'library') ? 'page' : undefined
                }
                active={page === n.id || (page === 'detail' && n.id === 'library')}
                label={<span className="nav-label">{n.label}</span>}
                leftSection={<Icon name={n.id} />}
                rightSection={
                  (n.id === 'library' || n.id === 'queue') && !appearance.collapsed ? (
                    <Badge
                      variant="light"
                      size="sm"
                      aria-hidden="true"
                      title={
                        n.id === 'library'
                          ? '会议库总数；— 表示尚未确定'
                          : '待关注任务，包含排队、处理中、失败、阻塞和中断；+ 表示还有后续页，— 表示尚未确定'
                      }
                    >
                      {counts[n.id]}
                      {n.id === 'queue' ? ' 待关注' : ''}
                    </Badge>
                  ) : undefined
                }
                onClick={(e) => {
                  e.preventDefault();
                  go(n.id);
                }}
              />
            </Tooltip>
          ))}
        </nav>
        <div className="sidebar-foot">
          <Anchor
            className="sidebar-status"
            href="#diagnostics"
            aria-label="查看后台服务状态"
            onClick={(e) => {
              e.preventDefault();
              go('diagnostics');
            }}
          >
            <Icon name="diagnostics" />
            <span className="sidebar-status-label">
              {info.data?.compatibility.state === 'compatible'
                ? '服务协议兼容'
                : '查看服务与依赖状态'}
            </span>
          </Anchor>
          <Text component="p">
            录音与转写留在本机
            <br />
            纪要使用专用 Pi 环境
          </Text>
        </div>
      </Paper>
      <div className="shell">
        <Paper component="header" withBorder p="sm" className="topbar">
          <span id="breadcrumb">工作空间 / {label}</span>
          <label className="theme-picker">
            外观
            <NativeSelect
              aria-label="外观主题"
              value={appearance.theme}
              onChange={(e) => appearance.setTheme(e.target.value as Theme)}
            >
              <option value="system">跟随系统</option>
              <option value="light">浅色</option>
              <option value="dark">深色</option>
            </NativeSelect>
          </label>
        </Paper>
        {appearance.warning && (
          <Text className="theme-warning" role="status" component="p">
            {appearance.warning}
          </Text>
        )}
        <RecordingBar />
        <ServiceBanner open={() => go('diagnostics')} />
        {info.data && !info.data.compatibility.writesAllowed && (
          <Alert className="compatibility-banner" role="status" color="orange">
            当前资料库为只读或服务不可用。查看“状态与诊断”修复；不会自动迁移个人资料或重启服务。
          </Alert>
        )}
        <main id="main" tabIndex={-1} ref={main}>
          <NoticeOutlet />
          {!!closeError && <ErrorNotice error={closeError} />}{' '}
          <div hidden={page !== 'library'}>
            <Library open={(id) => go('detail', id)} />
          </div>
          {page === 'library' ? null : page === 'detail' ? (
            <Detail
              key={meeting}
              id={meeting}
              back={() => go('library')}
              openMeeting={(id) => go('detail', id)}
            />
          ) : page === 'queue' ? (
            <Queue openMeeting={(id) => go('detail', id)} />
          ) : page === 'settings' ? (
            <Settings register={register} />
          ) : page === 'diagnostics' ? (
            <DiagnosticsPage />
          ) : (
            <Help />
          )}
        </main>
        <footer className="page-footer">
          <span>Open Meeting Memo</span>
          <span>本机工作空间 · {info.data ? `v${info.data.appVersion}` : '正在读取版本'}</span>
        </footer>
      </div>
      {pending && (
        <Modal
          title="设置尚未保存"
          close={() => {
            if (!saving) setPending(null);
          }}
          busy={saving}
        >
          <Text component="p">
            离开前可以保存当前配置、放弃草稿，或留在设置继续编辑。保存冲突时保留草稿，不会覆盖其他进程的修改。
          </Text>
          <div className="modal-actions">
            <Button
              disabled={saving}
              onClick={() => setPending(null)}
              type="button"
              variant="default"
            >
              留在设置
            </Button>
            <Button
              disabled={saving}
              onClick={() => {
                guard.current?.discard();
                const next = pending;
                setPending(null);
                next();
              }}
              type="button"
              variant="default"
            >
              放弃并离开
            </Button>
            <Button
              className="primary"
              disabled={saving}
              onClick={() => {
                setSaving(true);
                void (guard.current?.save() ?? Promise.resolve(true))
                  .then((ok) => {
                    if (ok) {
                      const next = pending;
                      setPending(null);
                      next();
                    }
                  })
                  .finally(() => setSaving(false));
              }}
              type="button"
              variant="filled"
            >
              保存并离开
            </Button>
          </div>
        </Modal>
      )}
    </>
  );
}
export function App({ bridge, client }: { bridge?: Bridge; client?: QueryClient }) {
  const [realBridge] = useState(() => bridge ?? createBridge());
  const [queryClient] = useState(
    () =>
      client ??
      new QueryClient({
        defaultOptions: {
          queries: { retry: false, staleTime: 1000, gcTime: 60000 },
          mutations: { retry: false },
        },
      }),
  );
  return (
    <AppearanceProvider>
      <BridgeContext.Provider value={realBridge}>
        <QueryClientProvider client={queryClient}>
          <Notices>
            <Shell />
          </Notices>
        </QueryClientProvider>
      </BridgeContext.Provider>
    </AppearanceProvider>
  );
}
