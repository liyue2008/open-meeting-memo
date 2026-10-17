import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useAction, useBridge } from '../app/context';
import { dateRange, dateTime, clock, type DatePreset } from '../app/dates';
import { ErrorNotice, Heading, Loading, SOURCES, STATUS, Status } from '../components/ui';
import { CreateMeeting, EditMeeting, Reprocess, RetainMeetings } from './meeting-actions';
import type {
  MeetingFilter,
  MeetingRow,
  MeetingSort,
  MeetingStatus,
  SourceKind,
  Visibility,
} from '../contracts.generated';
import { BridgeFailure } from '../bridge/client';
export function Library({ open }: { open: (id: string) => void }) {
  const bridge = useBridge();
  const action = useAction();
  const [query, setQuery] = useState('');
  const [search, setSearch] = useState('');
  const [scope, setScope] = useState<'title' | 'content'>('title');
  const [preset, setPreset] = useState<DatePreset>('7');
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [source, setSource] = useState<SourceKind | ''>('');
  const [status, setStatus] = useState<MeetingStatus | ''>('');
  const [sort, setSort] = useState<MeetingSort>('started_desc');
  const [visibility, setVisibility] = useState<Visibility>('active');
  const [pageSize, setPageSize] = useState(25);
  const [pages, setPages] = useState<Array<{ cursor: string | null; revision: string | null }>>([
    { cursor: null, revision: null },
  ]);
  const [pageNotice, setPageNotice] = useState('');
  const [selected, setSelected] = useState<MeetingRow[]>([]);
  const [dialog, setDialog] = useState<'create' | 'edit' | 'trash' | 'process' | null>(null);
  const [editing, setEditing] = useState<MeetingRow | null>(null);
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query), 300);
    return () => clearTimeout(timer);
  }, [query]);
  let range = null;
  let invalidDate = false;
  try {
    range = dateRange(preset, new Date(), from, to);
  } catch {
    invalidDate = true;
  }
  const filter: MeetingFilter = {
    query: search,
    scope,
    date: range,
    sources: source ? [source] : [],
    statuses: status ? [status] : [],
    sort,
    visibility,
  };
  const key = JSON.stringify({ filter, pageSize });
  useEffect(() => {
    setPages([{ cursor: null, revision: null }]);
    setSelected([]);
    setPageNotice('');
  }, [key]);
  const current = pages[pages.length - 1]!;
  const list = useQuery({
    queryKey: ['meeting_list', key, current],
    queryFn: ({ signal }) =>
      bridge.call(
        'meeting_list',
        { filter, cursor: current.cursor, pageSize, expectedRevision: current.revision },
        signal,
      ),
    enabled: !invalidDate,
    refetchInterval: pages.length === 1 ? 10000 : false,
  });
  useEffect(() => {
    setSelected([]);
  }, [list.data?.revision]);
  useEffect(() => {
    if (
      list.error instanceof BridgeFailure &&
      list.error.detail.code === 'conflict' &&
      pages.length > 1
    ) {
      setPages([{ cursor: null, revision: null }]);
      setPageNotice('会议列表已变化，已返回第一页。请重新核对选择后继续。');
    }
  }, [list.error, pages.length]);
  const rows = list.data?.items ?? [];
  const select = (row: MeetingRow, checked: boolean) =>
    setSelected((s) =>
      checked
        ? [...s.filter((r) => r.meetingId !== row.meetingId), row].slice(0, 100)
        : s.filter((r) => r.meetingId !== row.meetingId),
    );
  const close = () => {
    setDialog(null);
    setSelected([]);
  };
  return (
    <>
      <Heading title="会议库" text="保留讨论原声，让结论和下一步随时可找。">
        <button className="primary" onClick={() => setDialog('create')}>
          ＋ 新建会议
        </button>
      </Heading>
      <div className="overview-strip">
        <section className="welcome-card">
          <h2>每次讨论，都值得被记住。</h2>
          <p>录音与逐字稿留在本机；纪要使用你配置的 Pi 模型。云端模型会收到逐字稿文本。</p>
        </section>
        <section className="queue-card">
          <h3>真实处理状态</h3>
          <p>重新处理保留旧产物；回收站不永久删除任何录音。</p>
          <p>
            {list.data
              ? `${list.data.search.indexed} 场已建立检索投影，${list.data.search.pending} 场待处理`
              : '正在读取检索状态'}
          </p>
        </section>
      </div>
      {pageNotice && (
        <p role="status" className="notice">
          {pageNotice}
        </p>
      )}
      <section className="panel" aria-label="会议列表">
        <div className="filter-top">
          <label className="searchbox">
            <span aria-hidden="true">⌕</span>
            <input
              aria-label="搜索会议"
              placeholder="搜索会议标题或内容…"
              maxLength={1024}
              value={query}
              onChange={(e) => setQuery(Array.from(e.target.value).slice(0, 512).join(''))}
            />
          </label>
          <label>
            搜索范围
            <select value={scope} onChange={(e) => setScope(e.target.value as typeof scope)}>
              <option value="title">标题</option>
              <option value="content">标题、逐字稿和纪要</option>
            </select>
          </label>
          <label>
            日期
            <select value={preset} onChange={(e) => setPreset(e.target.value as DatePreset)}>
              <option value="today">今日</option>
              <option value="week">本周（周一起）</option>
              <option value="3">最近 3 天</option>
              <option value="7">最近 7 天</option>
              <option value="all">全部时间</option>
              <option value="custom">自定义</option>
            </select>
          </label>
          {preset === 'custom' && (
            <>
              <input
                aria-label="开始日期"
                type="date"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
              />
              <input
                aria-label="结束日期（含）"
                type="date"
                value={to}
                onChange={(e) => setTo(e.target.value)}
              />
            </>
          )}
        </div>
        <div className="filter-bottom">
          <label>
            来源
            <select value={source} onChange={(e) => setSource(e.target.value as typeof source)}>
              <option value="">全部来源</option>
              {Object.entries(SOURCES).map(([k, v]) => (
                <option key={k} value={k}>
                  {v}
                </option>
              ))}
            </select>
          </label>
          <label>
            状态
            <select value={status} onChange={(e) => setStatus(e.target.value as typeof status)}>
              <option value="">全部状态</option>
              {Object.entries(STATUS).map(([k, v]) => (
                <option key={k} value={k}>
                  {v}
                </option>
              ))}
            </select>
          </label>
          <label>
            位置
            <select
              value={visibility}
              onChange={(e) => setVisibility(e.target.value as Visibility)}
            >
              <option value="active">会议库</option>
              <option value="trash">回收站</option>
              <option value="all">全部（含回收站）</option>
            </select>
          </label>
          <label className="sort">
            排序
            <select value={sort} onChange={(e) => setSort(e.target.value as MeetingSort)}>
              <option value="started_desc">开始时间 ↓</option>
              <option value="started_asc">开始时间 ↑</option>
              <option value="duration_desc">时长 ↓</option>
              <option value="duration_asc">时长 ↑</option>
            </select>
          </label>
        </div>
        {selected.length > 0 && (
          <div className="bulk-bar">
            <span>已选 {selected.length} 场</span>
            <button onClick={() => setSelected([])}>取消选择</button>
            <button
              disabled={!selected.every((m) => m.capabilities.reprocess)}
              onClick={() => setDialog('process')}
            >
              重新处理
            </button>
            <button
              disabled={
                !selected.every((m) =>
                  m.trashed ? m.capabilities.restore : m.capabilities.trash,
                ) || selected.some((m) => m.trashed !== selected[0]?.trashed)
              }
              onClick={() => setDialog('trash')}
            >
              {selected[0]?.trashed ? '恢复' : '移入回收站'}
            </button>
            <button
              disabled={action.busy}
              onClick={() =>
                void action.run(async () => {
                  const r = await bridge.call('meeting_batch_export', {
                    meetingIds: selected.map((m) => m.meetingId),
                    stage: 'summary',
                    format: 'markdown',
                  });
                  return r.outcome === 'cancelled' ? '已取消导出' : '纪要已导出';
                })
              }
            >
              批量导出纪要
            </button>
          </div>
        )}
        {invalidDate ? (
          <p role="alert" className="empty-state">
            请选择有效的日期区间。
          </p>
        ) : list.isPending ? (
          <Loading />
        ) : list.error ? (
          <ErrorNotice error={list.error} retry={() => void list.refetch()} />
        ) : (
          <>
            {!list.data.search.complete && (
              <div className="notice">内容索引尚未完成，当前结果可能不完整。</div>
            )}
            {rows.length === 0 ? (
              <div className="empty-state">
                <h2>没有符合条件的会议</h2>
                <p>调整日期和筛选，或新建录音 / 导入已有音频。</p>
              </div>
            ) : (
              <div className="table-scroll">
                <table>
                  <thead>
                    <tr>
                      <th>
                        <input
                          type="checkbox"
                          aria-label="选择本页全部会议"
                          checked={
                            rows.length > 0 &&
                            rows.every((r) => selected.some((s) => s.meetingId === r.meetingId))
                          }
                          onChange={(e) => setSelected(e.target.checked ? rows : [])}
                        />
                      </th>
                      <th>会议</th>
                      <th>开始时间</th>
                      <th>时长</th>
                      <th>来源</th>
                      <th>状态</th>
                      <th>操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((row) => (
                      <tr key={row.meetingId}>
                        <td>
                          <input
                            type="checkbox"
                            aria-label={`选择 ${row.metadata.title}`}
                            checked={selected.some((s) => s.meetingId === row.meetingId)}
                            onChange={(e) => select(row, e.target.checked)}
                          />
                        </td>
                        <td>
                          <button
                            className="text-button meeting-title"
                            onClick={() => open(row.meetingId)}
                          >
                            {row.metadata.title}
                          </button>
                          {row.trashed && <small> · 回收站</small>}
                          <div className="tag-list">
                            {row.metadata.tags.map((t) => (
                              <span className="tag" key={t}>
                                {t}
                              </span>
                            ))}
                          </div>
                        </td>
                        <td>
                          {row.timeSource === 'estimated' ? '约 ' : ''}
                          {dateTime(row.startedAtMs)}
                          {row.timeSource === 'unknown' && <small>（未知）</small>}
                        </td>
                        <td>{row.durationMs === null ? '未知' : clock(row.durationMs)}</td>
                        <td>{SOURCES[row.source]}</td>
                        <td>
                          <Status status={row.status} />
                        </td>
                        <td>
                          <button
                            disabled={!row.capabilities.edit}
                            title={row.capabilities.reason ?? undefined}
                            onClick={() => {
                              setEditing(row);
                              setDialog('edit');
                            }}
                          >
                            编辑
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
            <div className="pagination">
              <span>
                {list.data.total === null ? '总数尚未确定' : `共 ${list.data.total} 场`} · 第{' '}
                {pages.length} 页
              </span>
              <label>
                每页
                <select value={pageSize} onChange={(e) => setPageSize(Number(e.target.value))}>
                  {[25, 50, 100].map((s) => (
                    <option key={s}>{s}</option>
                  ))}
                </select>
              </label>
              <button
                disabled={pages.length === 1 || list.isFetching}
                onClick={() => setPages((p) => p.slice(0, -1))}
              >
                上一页
              </button>
              <button
                disabled={!list.data.nextCursor || list.isFetching}
                onClick={() =>
                  setPages((p) => [
                    ...p,
                    { cursor: list.data.nextCursor, revision: list.data.revision },
                  ])
                }
              >
                下一页
              </button>
            </div>
          </>
        )}
      </section>
      {dialog === 'create' && <CreateMeeting close={close} />}{' '}
      {dialog === 'edit' && editing && <EditMeeting meeting={editing} close={close} />}{' '}
      {dialog === 'trash' && (
        <RetainMeetings meetings={selected} restore={!!selected[0]?.trashed} close={close} />
      )}{' '}
      {dialog === 'process' && <Reprocess meetings={selected} close={close} />}
    </>
  );
}
