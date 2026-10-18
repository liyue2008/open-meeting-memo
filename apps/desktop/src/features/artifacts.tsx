import { Alert, Button, Paper, Tabs, Text, TextInput, Title, Typography } from '@mantine/core';
import { useEffect, useState } from 'react';
import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query';
import { BridgeFailure } from '../bridge/client';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { useAction, useBridge } from '../app/context';
import { clock, dateTime } from '../app/dates';
import { ARTIFACT_STATUS, STATUS, Disclosure, ErrorNotice, Loading } from '../components/ui';
import { isActiveMeetingStatus, isActiveTaskStatus } from './meeting-task-activity';
import type { ArtifactRef, ArtifactVersion, MeetingRow, Stage } from '../contracts.generated';

export function timestampMs(value: string): number | null {
  const match = /^(?:(\d{1,2}):)?(\d{1,2}):(\d{2})$/.exec(value);
  if (!match || Number(match[2]) >= 60 || Number(match[3]) >= 60) return null;
  return ((Number(match[1] ?? 0) * 60 + Number(match[2])) * 60 + Number(match[3])) * 1000;
}
export function readOnlyActions(text: string): string[] {
  const lines = text.split('\n');
  const actions: string[] = [];
  let inSection = false;
  for (const line of lines) {
    if (/^#{1,6}\s/.test(line)) {
      inSection = /行动项|待办|下一步|action items/i.test(line);
      continue;
    }
    if (inSection) {
      const item = /^\s*(?:[-*+] |\d+\. )(?:\[[ xX]\]\s*)?(.+)$/.exec(line);
      if (item?.[1]) actions.push(item[1]);
    }
  }
  return actions;
}
interface MarkdownNode {
  type: string;
  value?: string;
  url?: string;
  children?: MarkdownNode[];
}
function timestampLinks() {
  return (tree: MarkdownNode) => {
    const walk = (node: MarkdownNode) => {
      if (!node.children || ['link', 'image', 'code', 'inlineCode', 'html'].includes(node.type))
        return;
      node.children = node.children.flatMap((child) => {
        if (child.type !== 'text' || !child.value) {
          walk(child);
          return [child];
        }
        const value = child.value;
        const result: MarkdownNode[] = [];
        let offset = 0;
        for (const match of value.matchAll(/\[((?:\d{1,2}:)?\d{1,2}:\d{2})\]/g)) {
          const time = timestampMs(match[1]!);
          if (time === null) continue;
          const index = match.index;
          if (index > offset) result.push({ type: 'text', value: value.slice(offset, index) });
          result.push({
            type: 'link',
            url: `#t=${time / 1000}`,
            children: [{ type: 'text', value: match[0] }],
          });
          offset = index + match[0].length;
        }
        if (!offset) return [child];
        if (offset < value.length) result.push({ type: 'text', value: value.slice(offset) });
        return result;
      });
    };
    walk(tree);
  };
}
export function SafeMarkdown({
  text,
  duration,
  locate,
}: {
  text: string;
  duration: number | null;
  locate: (time: number) => void;
}) {
  return (
    <Typography className="markdown">
      <Markdown
        remarkPlugins={[remarkGfm, timestampLinks]}
        skipHtml
        components={{
          img: () => null,
          a: ({ children, href }) => {
            const match = /^#t=(\d+)$/.exec(href ?? '');
            const time = match ? Number(match[1]) * 1000 : null;
            return time !== null && duration !== null && time >= 0 && time <= duration ? (
              <Button
                className="text-button"
                onClick={() => locate(time)}
                type="button"
                variant="subtle"
              >
                {children}
              </Button>
            ) : (
              <span>{children}</span>
            );
          },
          input: () => null,
        }}
      >
        {text}
      </Markdown>
    </Typography>
  );
}
function cursorConflict(error: unknown): boolean {
  return error instanceof BridgeFailure && error.detail.code === 'conflict';
}
function ArtifactBody({
  artifact,
  snapshot,
  duration,
  locate,
}: {
  artifact: ArtifactRef;
  snapshot: string;
  duration: number | null;
  locate: (time: number) => void;
}) {
  const bridge = useBridge();
  const action = useAction();
  const client = useQueryClient();
  const [search, setSearch] = useState('');
  const chunks = useInfiniteQuery({
    queryKey: ['artifact_read', artifact, snapshot],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) =>
      bridge.call('artifact_read', { artifact, cursor: pageParam, maxBytes: 262144 }, signal),
    getNextPageParam: (p) => p.nextCursor ?? undefined,
  });
  useEffect(() => {
    if (cursorConflict(chunks.error) && chunks.data) {
      // A cursor belongs to one snapshot: discard all pages before reading a new head.
      void client.resetQueries({ queryKey: ['artifact_read', artifact, snapshot], exact: true });
      void client.invalidateQueries({ queryKey: ['artifact_versions', artifact.meetingId] });
    }
  }, [chunks.error, client, artifact.meetingId, artifact.taskId, artifact.stage, snapshot]);
  const data = chunks.data?.pages ?? [];
  const lines = data.flatMap((c) => c.lines);
  const text = data.map((c) => c.text).join('');
  const status = data[0]?.status;
  const actions = readOnlyActions(text);
  return (
    <>
      <div className="document-toolbar">
        <div className="button-row">
          <Button
            disabled={action.busy}
            onClick={() =>
              void action.run(async () => {
                const r = await bridge.call('artifact_copy', { artifact });
                return r.acknowledged ? '已复制完整产物' : '未复制';
              })
            }
            type="button"
            variant="default"
          >
            复制完整内容
          </Button>
          <Button
            disabled={action.busy}
            onClick={() =>
              void action.run(async () => {
                const r = await bridge.call('artifact_export', {
                  artifact,
                  format: artifact.stage === 'summary' ? 'markdown' : 'text',
                });
                return r.outcome === 'saved' ? '内容已导出' : '已取消导出';
              })
            }
            type="button"
            variant="default"
          >
            导出
          </Button>
        </div>
        {artifact.stage === 'transcribe' && (
          <label>
            当前已载入逐字稿内搜索
            <TextInput value={search} onChange={(e) => setSearch(e.target.value)} maxLength={512} />
          </label>
        )}
      </div>
      {chunks.isPending ? (
        <Loading />
      ) : chunks.error ? (
        <ErrorNotice error={chunks.error} retry={() => void chunks.refetch()} />
      ) : (
        <>
          {status !== 'committed' && (
            <Alert className="notice" variant="light" role="status">
              {status === 'partial'
                ? '部分逐字稿：处理尚未完成，当前内容不是终稿。'
                : status === 'historical'
                  ? '历史产物：保留供查阅，不代表当前逐字稿对应的纪要。'
                  : `产物状态：${status ? ARTIFACT_STATUS[status] : '未知'}`}
            </Alert>
          )}
          {artifact.stage === 'transcribe' ? (
            <div className="transcript-list">
              {lines
                .filter(
                  (l) => !search || l.text.toLocaleLowerCase().includes(search.toLocaleLowerCase()),
                )
                .map((line, index) => (
                  <div className="transcript-line" key={`${line.startMs}-${index}`}>
                    <Button
                      className="timestamp text-button"
                      disabled={duration === null || line.startMs > duration}
                      onClick={() => locate(line.startMs)}
                      type="button"
                      variant="subtle"
                    >
                      {clock(line.startMs)}
                    </Button>
                    <span className="speaker">
                      {line.speaker === 'mic'
                        ? '麦克风'
                        : line.speaker === 'system'
                          ? '系统音频'
                          : line.speaker === 'mix'
                            ? '混音'
                            : '未知声源'}
                    </span>
                    <Text component="p">{line.text}</Text>
                  </div>
                ))}
              {lines.length === 0 && (
                <Text className="empty-state" component="p">
                  {status === 'partial' ? '还没有已完成的转写片段。' : '当前产物没有转写行。'}
                </Text>
              )}
            </div>
          ) : (
            <>
              <SafeMarkdown text={text} duration={duration} locate={locate} />
              <section className="action-items">
                <Title order={3}>行动项 · 只读</Title>
                {actions.length ? (
                  <ul>
                    {actions.map((item, index) => (
                      <li key={index}>{item}</li>
                    ))}
                  </ul>
                ) : (
                  <Text className="muted" component="p" c="dimmed">
                    纪要中没有可识别的结构化行动项。完整纪要仍在上方。
                  </Text>
                )}
              </section>
            </>
          )}
          {chunks.hasNextPage && (
            <Button
              disabled={chunks.isFetchingNextPage || data.length >= 32}
              onClick={() => void chunks.fetchNextPage()}
              type="button"
              variant="default"
            >
              载入更多内容
            </Button>
          )}
          {data.length >= 32 && chunks.hasNextPage && (
            <Text component="p">已达到窗口阅读上限，请导出完整产物。</Text>
          )}
        </>
      )}
    </>
  );
}
export function Artifacts({
  meeting,
  partial,
  locate,
  onTranscript,
}: {
  meeting: MeetingRow;
  partial: boolean;
  locate: (time: number) => void;
  onTranscript: (taskId: string | null) => void;
}) {
  const bridge = useBridge();
  const client = useQueryClient();
  const [tab, setTab] = useState<Stage | 'versions'>('summary');
  const [chosen, setChosen] = useState<ArtifactVersion | null>(null);
  const versionsKey = [
    'artifact_versions',
    meeting.meetingId,
    meeting.status,
    partial,
    meeting.currentTranscriptTaskId,
    meeting.currentSummaryTaskId,
  ];
  const versions = useInfiniteQuery({
    queryKey: versionsKey,
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) =>
      bridge.call(
        'artifact_versions',
        { meetingId: meeting.meetingId, cursor: pageParam, pageSize: 100 },
        signal,
      ),
    getNextPageParam: (p) => p.nextCursor ?? undefined,
    refetchInterval: (query) => {
      const items = query.state.data?.pages.flatMap((p) => p.items);
      const active =
        ['recording', 'unsealed'].includes(meeting.status) ||
        (items?.length
          ? items.some((v) => isActiveTaskStatus(v.taskStatus))
          : isActiveMeetingStatus(meeting.status));
      return active ? 2000 : false;
    },
    refetchIntervalInBackground: true,
  });
  useEffect(() => {
    if (cursorConflict(versions.error) && versions.data) {
      void client.resetQueries({ queryKey: versionsKey, exact: true });
    }
  }, [
    versions.error,
    client,
    meeting.meetingId,
    meeting.status,
    partial,
    meeting.currentTranscriptTaskId,
    meeting.currentSummaryTaskId,
  ]);
  const available = versions.data?.pages.flatMap((p) => p.items) ?? [];
  const defaultId =
    tab === 'summary' ? meeting.currentSummaryTaskId : meeting.currentTranscriptTaskId;
  const selected =
    chosen?.artifact.stage === tab
      ? (available.find(
          (v) => v.artifact.taskId === chosen.artifact.taskId && v.artifact.stage === tab,
        ) ?? chosen)
      : (available.find((v) => v.artifact.stage === tab && v.current) ??
        available.find((v) => v.artifact.stage === tab && v.artifact.taskId === defaultId) ??
        (tab === 'transcribe'
          ? available.find((v) => v.artifact.stage === 'transcribe' && v.status === 'partial')
          : undefined));
  const artifact: ArtifactRef | null =
    selected?.artifact ??
    (defaultId && tab !== 'versions'
      ? { meetingId: meeting.meetingId, taskId: defaultId, stage: tab }
      : null);
  useEffect(() => {
    onTranscript(
      tab === 'transcribe' && selected && ['committed', 'historical'].includes(selected.status)
        ? selected.artifact.taskId
        : meeting.currentTranscriptTaskId,
    );
  }, [tab, selected?.artifact.taskId, selected?.status, meeting.currentTranscriptTaskId]);
  return (
    <Paper className="panel artifact-panel" withBorder p="md" component="section">
      <Tabs
        value={tab}
        onChange={(next) => {
          if (next) {
            setTab(next as Stage | 'versions');
            setChosen(null);
          }
        }}
        keepMounted={false}
      >
        <Tabs.List aria-label="会议内容">
          {(['summary', 'transcribe', 'versions'] as const).map((t) => (
            <Tabs.Tab
              key={t}
              value={t}
              onKeyDown={(event) => {
                if (event.key === 'Home' || event.key === 'End') {
                  setTab(event.key === 'Home' ? 'summary' : 'versions');
                  setChosen(null);
                }
              }}
            >
              {t === 'summary' ? '会议纪要' : t === 'transcribe' ? '逐字稿' : '处理版本'}
            </Tabs.Tab>
          ))}
        </Tabs.List>
        {(['summary', 'transcribe', 'versions'] as const).map((panel) => (
          <Tabs.Panel key={panel} value={panel} className="document-content" pt="md">
            {tab === panel && (
              <>
                {versions.error && (
                  <ErrorNotice error={versions.error} retry={() => void versions.refetch()} />
                )}
                {tab === 'versions' ? (
                  <>
                    <Text className="muted" component="p" c="dimmed">
                      显示真实处理次数、产物校验和与来源。失败的新版本不会覆盖旧的已提交产物。
                    </Text>
                    {versions.isPending && <Loading />}
                    {available.map((v) => (
                      <div className="version-row" key={`${v.artifact.taskId}-${v.artifact.stage}`}>
                        <div>
                          <strong>
                            第 {v.attempt} 次 · {v.artifact.stage === 'summary' ? '纪要' : '逐字稿'}
                          </strong>{' '}
                          · {v.current ? '当前' : '历史'} · {ARTIFACT_STATUS[v.status]} /{' '}
                          {STATUS[v.taskStatus]}
                          <Text component="p">
                            {dateTime(v.createdAtMs)} · 模型 {v.modelVersion}
                          </Text>
                          <Disclosure title="校验与输入血缘">
                            <dl>
                              <dt>产物 SHA-256</dt>
                              <dd>{v.sha256 ?? '尚未提交'}</dd>
                              <dt>配置 SHA-256</dt>
                              <dd>{v.configHash}</dd>
                              <dt>来源逐字稿</dt>
                              <dd>
                                {v.sourceTranscriptTaskId ?? '无'} /{' '}
                                {v.sourceTranscriptHash ?? '无'}
                              </dd>
                            </dl>
                          </Disclosure>
                        </div>
                        <Button
                          disabled={!['committed', 'historical', 'partial'].includes(v.status)}
                          onClick={() => {
                            setChosen(v);
                            setTab(v.artifact.stage);
                          }}
                          type="button"
                          variant="default"
                        >
                          读取此版本
                        </Button>
                      </div>
                    ))}
                    {!versions.isPending && available.length === 0 && (
                      <Text className="empty-state" component="p">
                        尚无处理版本。
                      </Text>
                    )}
                    {versions.hasNextPage && (
                      <Button
                        disabled={versions.isFetchingNextPage}
                        onClick={() => void versions.fetchNextPage()}
                        type="button"
                        variant="default"
                      >
                        更多版本
                      </Button>
                    )}
                  </>
                ) : (
                  <>
                    {selected && (
                      <Alert className="notice" variant="light" role="status">
                        第 {selected.attempt} 次处理 · {selected.current ? '当前产物' : '历史产物'}{' '}
                        · {ARTIFACT_STATUS[selected.status]}
                        {!selected.current && '；重新处理仍保留此版本'}
                      </Alert>
                    )}
                    {artifact ? (
                      <ArtifactBody
                        key={`${artifact.taskId}-${artifact.stage}`}
                        artifact={artifact}
                        snapshot={`${selected?.status ?? 'missing'}:${selected?.sha256 ?? ''}`}
                        duration={meeting.durationMs}
                        locate={locate}
                      />
                    ) : (
                      <Text className="empty-state" component="p">
                        {tab === 'transcribe' && (partial || isActiveMeetingStatus(meeting.status))
                          ? '正在准备部分逐字稿；可在处理版本中查看已完成片段。'
                          : '尚无对应的有效产物。处理失败时可以到处理版本中查阅旧结果。'}
                      </Text>
                    )}
                  </>
                )}
              </>
            )}
          </Tabs.Panel>
        ))}
      </Tabs>
    </Paper>
  );
}
