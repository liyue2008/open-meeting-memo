import {
  Alert,
  Badge,
  Button,
  Checkbox,
  NativeSelect,
  Pagination,
  Paper,
  Table,
  Text,
  TextInput,
  Title,
} from '@mantine/core';
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
        <Button
          className="primary"
          onClick={() => setDialog('create')}
          type="button"
          variant="filled"
        >
          ＋ 新建会议
        </Button>
      </Heading>
      <Paper withBorder p="sm" mb="md">
        <Text size="sm" c="dimmed">
          录音与逐字稿留在本机；纪要使用配置的 Pi 模型，云端模型会收到逐字稿文本。
        </Text>
        <Text size="sm" c="dimmed">
          重新处理保留旧产物；回收站不永久删除录音。
          {list.data
            ? `检索：${list.data.search.indexed} 场已建立检索投影，${list.data.search.pending} 场待处理。`
            : '正在读取检索状态。'}
        </Text>
      </Paper>
      {pageNotice && (
        <Alert role="status" className="notice" variant="light">
          {pageNotice}
        </Alert>
      )}
      <Paper className="panel" aria-label="会议列表" withBorder p="md" component="section">
        <div className="filter-top">
          <label className="searchbox">
            <span aria-hidden="true">⌕</span>
            <TextInput
              aria-label="搜索会议"
              placeholder="搜索会议标题或内容…"
              maxLength={1024}
              value={query}
              onChange={(e) => setQuery(Array.from(e.target.value).slice(0, 512).join(''))}
            />
          </label>
          <label>
            搜索范围
            <NativeSelect value={scope} onChange={(e) => setScope(e.target.value as typeof scope)}>
              <option value="title">标题</option>
              <option value="content">标题、逐字稿和纪要</option>
            </NativeSelect>
          </label>
          <label>
            日期
            <NativeSelect value={preset} onChange={(e) => setPreset(e.target.value as DatePreset)}>
              <option value="today">今日</option>
              <option value="week">本周（周一起）</option>
              <option value="3">最近 3 天</option>
              <option value="7">最近 7 天</option>
              <option value="all">全部时间</option>
              <option value="custom">自定义</option>
            </NativeSelect>
          </label>
          {preset === 'custom' && (
            <>
              <TextInput
                aria-label="开始日期"
                type="date"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
              />
              <TextInput
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
            <NativeSelect
              value={source}
              onChange={(e) => setSource(e.target.value as typeof source)}
            >
              <option value="">全部来源</option>
              {Object.entries(SOURCES).map(([k, v]) => (
                <option key={k} value={k}>
                  {v}
                </option>
              ))}
            </NativeSelect>
          </label>
          <label>
            状态
            <NativeSelect
              value={status}
              onChange={(e) => setStatus(e.target.value as typeof status)}
            >
              <option value="">全部状态</option>
              {Object.entries(STATUS).map(([k, v]) => (
                <option key={k} value={k}>
                  {v}
                </option>
              ))}
            </NativeSelect>
          </label>
          <label>
            位置
            <NativeSelect
              value={visibility}
              onChange={(e) => setVisibility(e.target.value as Visibility)}
            >
              <option value="active">会议库</option>
              <option value="trash">回收站</option>
              <option value="all">全部（含回收站）</option>
            </NativeSelect>
          </label>
          <label className="sort">
            排序
            <NativeSelect value={sort} onChange={(e) => setSort(e.target.value as MeetingSort)}>
              <option value="started_desc">开始时间 ↓</option>
              <option value="started_asc">开始时间 ↑</option>
              <option value="duration_desc">时长 ↓</option>
              <option value="duration_asc">时长 ↑</option>
            </NativeSelect>
          </label>
        </div>
        {selected.length > 0 && (
          <div className="bulk-bar">
            <span>已选 {selected.length} 场</span>
            <Button onClick={() => setSelected([])} type="button" variant="default">
              取消选择
            </Button>
            <Button
              disabled={!selected.every((m) => m.capabilities.reprocess)}
              onClick={() => setDialog('process')}
              type="button"
              variant="default"
            >
              重新处理
            </Button>
            <Button
              disabled={
                !selected.every((m) =>
                  m.trashed ? m.capabilities.restore : m.capabilities.trash,
                ) || selected.some((m) => m.trashed !== selected[0]?.trashed)
              }
              onClick={() => setDialog('trash')}
              type="button"
              variant="default"
            >
              {selected[0]?.trashed ? '恢复' : '移入回收站'}
            </Button>
            <Button
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
              type="button"
              variant="default"
            >
              批量导出纪要
            </Button>
          </div>
        )}
        {invalidDate ? (
          <Text role="alert" className="empty-state" component="p">
            请选择有效的日期区间。
          </Text>
        ) : list.isPending ? (
          <Loading />
        ) : list.error ? (
          <ErrorNotice error={list.error} retry={() => void list.refetch()} />
        ) : (
          <>
            {!list.data.search.complete && (
              <Alert className="notice" variant="light" role="status">
                内容索引尚未完成，当前结果可能不完整。
              </Alert>
            )}
            {rows.length === 0 ? (
              <div className="empty-state">
                <Title order={2}>没有符合条件的会议</Title>
                <Text component="p">调整日期和筛选，或新建录音 / 导入已有音频。</Text>
              </div>
            ) : (
              <Table.ScrollContainer className="table-scroll" minWidth={760} type="native">
                <Table highlightOnHover verticalSpacing="md">
                  <Table.Thead>
                    <Table.Tr>
                      <Table.Th>
                        <Checkbox
                          type="checkbox"
                          aria-label="选择本页全部会议"
                          checked={
                            rows.length > 0 &&
                            rows.every((r) => selected.some((s) => s.meetingId === r.meetingId))
                          }
                          onChange={(e) => setSelected(e.target.checked ? rows : [])}
                        />
                      </Table.Th>
                      <Table.Th>会议</Table.Th>
                      <Table.Th>开始时间</Table.Th>
                      <Table.Th>时长</Table.Th>
                      <Table.Th>来源</Table.Th>
                      <Table.Th>状态</Table.Th>
                      <Table.Th>操作</Table.Th>
                    </Table.Tr>
                  </Table.Thead>
                  <Table.Tbody>
                    {rows.map((row) => (
                      <Table.Tr key={row.meetingId}>
                        <Table.Td>
                          <Checkbox
                            type="checkbox"
                            aria-label={`选择 ${row.metadata.title}`}
                            checked={selected.some((s) => s.meetingId === row.meetingId)}
                            onChange={(e) => select(row, e.target.checked)}
                          />
                        </Table.Td>
                        <Table.Td>
                          <Button
                            className="text-button meeting-title"
                            onClick={() => open(row.meetingId)}
                            type="button"
                            variant="subtle"
                          >
                            {row.metadata.title}
                          </Button>
                          {row.trashed && (
                            <Text component="small" size="sm" c="dimmed">
                              {' '}
                              · 回收站
                            </Text>
                          )}
                          <div className="tag-list">
                            {row.metadata.tags.map((t) => (
                              <Badge className="tag" key={t}>
                                {t}
                              </Badge>
                            ))}
                          </div>
                        </Table.Td>
                        <Table.Td>
                          {row.timeSource === 'estimated' ? '约 ' : ''}
                          {dateTime(row.startedAtMs)}
                          {row.timeSource === 'unknown' && (
                            <Text component="small" size="sm" c="dimmed">
                              （未知）
                            </Text>
                          )}
                        </Table.Td>
                        <Table.Td>
                          {row.durationMs === null ? '未知' : clock(row.durationMs)}
                        </Table.Td>
                        <Table.Td>{SOURCES[row.source]}</Table.Td>
                        <Table.Td>
                          <Status status={row.status} />
                        </Table.Td>
                        <Table.Td>
                          <Button
                            disabled={!row.capabilities.edit}
                            title={row.capabilities.reason ?? undefined}
                            onClick={() => {
                              setEditing(row);
                              setDialog('edit');
                            }}
                            type="button"
                            variant="default"
                          >
                            编辑
                          </Button>
                        </Table.Td>
                      </Table.Tr>
                    ))}
                  </Table.Tbody>
                </Table>
              </Table.ScrollContainer>
            )}
            <div className="pagination">
              <span>
                {list.data.total === null ? '总数尚未确定' : `共 ${list.data.total} 场`} · 第{' '}
                {pages.length} 页
              </span>
              <label>
                每页
                <NativeSelect
                  value={pageSize}
                  onChange={(e) => setPageSize(Number(e.target.value))}
                >
                  {[25, 50, 100].map((s) => (
                    <option key={s}>{s}</option>
                  ))}
                </NativeSelect>
              </label>
              <Pagination.Root
                value={pages.length}
                total={pages.length + (list.data.nextCursor ? 1 : 0)}
              >
                <div className="button-row">
                  <Pagination.Previous
                    aria-label="上一页"
                    disabled={pages.length === 1 || list.isFetching}
                    onClick={() => setPages((p) => p.slice(0, -1))}
                  />
                  <Pagination.Next
                    aria-label="下一页"
                    disabled={!list.data.nextCursor || list.isFetching}
                    onClick={() =>
                      setPages((p) => [
                        ...p,
                        { cursor: list.data.nextCursor, revision: list.data.revision },
                      ])
                    }
                  />
                </div>
              </Pagination.Root>
            </div>
          </>
        )}
      </Paper>
      {dialog === 'create' && <CreateMeeting close={close} />}{' '}
      {dialog === 'edit' && editing && <EditMeeting meeting={editing} close={close} />}{' '}
      {dialog === 'trash' && (
        <RetainMeetings meetings={selected} restore={!!selected[0]?.trashed} close={close} />
      )}{' '}
      {dialog === 'process' && <Reprocess meetings={selected} close={close} />}
    </>
  );
}
