import { useEffect, useState } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { useAction, useBridge } from '../app/context';
import { clock, dateTime } from '../app/dates';
import { ARTIFACT_STATUS, STATUS, ErrorNotice, Loading } from '../components/ui';
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
    <div className="markdown">
      <Markdown
        remarkPlugins={[remarkGfm, timestampLinks]}
        skipHtml
        components={{
          img: () => null,
          a: ({ children, href }) => {
            const match = /^#t=(\d+)$/.exec(href ?? '');
            const time = match ? Number(match[1]) * 1000 : null;
            return time !== null && duration !== null && time >= 0 && time <= duration ? (
              <button className="text-button" onClick={() => locate(time)}>
                {children}
              </button>
            ) : (
              <span>{children}</span>
            );
          },
          input: () => null,
        }}
      >
        {text}
      </Markdown>
    </div>
  );
}
function ArtifactBody({
  artifact,
  duration,
  locate,
}: {
  artifact: ArtifactRef;
  duration: number | null;
  locate: (time: number) => void;
}) {
  const bridge = useBridge();
  const action = useAction();
  const [search, setSearch] = useState('');
  const chunks = useInfiniteQuery({
    queryKey: ['artifact_read', artifact],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) =>
      bridge.call('artifact_read', { artifact, cursor: pageParam, maxBytes: 262144 }, signal),
    getNextPageParam: (p) => p.nextCursor ?? undefined,
  });
  const data = chunks.data?.pages ?? [];
  const lines = data.flatMap((c) => c.lines);
  const text = data.map((c) => c.text).join('');
  const status = data[0]?.status;
  const actions = readOnlyActions(text);
  return (
    <>
      <div className="document-toolbar">
        <div className="button-row">
          <button
            disabled={action.busy}
            onClick={() =>
              void action.run(async () => {
                const r = await bridge.call('artifact_copy', { artifact });
                return r.acknowledged ? '已复制完整产物' : '未复制';
              })
            }
          >
            复制完整内容
          </button>
          <button
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
          >
            导出
          </button>
        </div>
        {artifact.stage === 'transcribe' && (
          <label>
            当前已载入逐字稿内搜索
            <input value={search} onChange={(e) => setSearch(e.target.value)} maxLength={512} />
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
            <div className="notice">
              {status === 'partial'
                ? '部分逐字稿：处理尚未完成，当前内容不是终稿。'
                : status === 'historical'
                  ? '历史产物：保留供查阅，不代表当前逐字稿对应的纪要。'
                  : `产物状态：${status ? ARTIFACT_STATUS[status] : '未知'}`}
            </div>
          )}
          {artifact.stage === 'transcribe' ? (
            <div className="transcript-list">
              {lines
                .filter(
                  (l) => !search || l.text.toLocaleLowerCase().includes(search.toLocaleLowerCase()),
                )
                .map((line, index) => (
                  <div className="transcript-line" key={`${line.startMs}-${index}`}>
                    <button
                      className="timestamp text-button"
                      disabled={duration === null || line.startMs > duration}
                      onClick={() => locate(line.startMs)}
                    >
                      {clock(line.startMs)}
                    </button>
                    <span className="speaker">
                      {line.speaker === 'mic'
                        ? '麦克风'
                        : line.speaker === 'system'
                          ? '系统音频'
                          : line.speaker === 'mix'
                            ? '混音'
                            : '未知声源'}
                    </span>
                    <p>{line.text}</p>
                  </div>
                ))}
              {lines.length === 0 && (
                <p className="empty-state">
                  {status === 'partial' ? '还没有已完成的转写片段。' : '当前产物没有转写行。'}
                </p>
              )}
            </div>
          ) : (
            <>
              <SafeMarkdown text={text} duration={duration} locate={locate} />
              <section className="action-items">
                <h3>行动项 · 只读</h3>
                {actions.length ? (
                  <ul>
                    {actions.map((item, index) => (
                      <li key={index}>{item}</li>
                    ))}
                  </ul>
                ) : (
                  <p className="muted">纪要中没有可识别的结构化行动项。完整纪要仍在上方。</p>
                )}
              </section>
            </>
          )}
          {chunks.hasNextPage && (
            <button
              disabled={chunks.isFetchingNextPage || data.length >= 32}
              onClick={() => void chunks.fetchNextPage()}
            >
              载入更多内容
            </button>
          )}
          {data.length >= 32 && chunks.hasNextPage && <p>已达到窗口阅读上限，请导出完整产物。</p>}
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
  const [tab, setTab] = useState<Stage | 'versions'>('summary');
  const [chosen, setChosen] = useState<ArtifactVersion | null>(null);
  const versions = useInfiniteQuery({
    queryKey: ['artifact_versions', meeting.meetingId],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) =>
      bridge.call(
        'artifact_versions',
        { meetingId: meeting.meetingId, cursor: pageParam, pageSize: 100 },
        signal,
      ),
    getNextPageParam: (p) => p.nextCursor ?? undefined,
  });
  const available = versions.data?.pages.flatMap((p) => p.items) ?? [];
  const defaultId =
    tab === 'summary' ? meeting.currentSummaryTaskId : meeting.currentTranscriptTaskId;
  const selected =
    chosen?.artifact.stage === tab
      ? chosen
      : (available.find((v) => v.artifact.stage === tab && v.artifact.taskId === defaultId) ??
        (tab === 'transcribe' && partial
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
    <section className="panel artifact-panel">
      <div className="tabs" role="tablist" aria-label="会议内容">
        {(['summary', 'transcribe', 'versions'] as const).map((t) => (
          <button
            key={t}
            role="tab"
            aria-selected={tab === t}
            tabIndex={tab === t ? 0 : -1}
            onKeyDown={(e) => {
              const tabs = ['summary', 'transcribe', 'versions'] as const;
              const index = tabs.indexOf(t);
              const next =
                e.key === 'ArrowRight'
                  ? tabs[(index + 1) % tabs.length]
                  : e.key === 'ArrowLeft'
                    ? tabs[(index + tabs.length - 1) % tabs.length]
                    : e.key === 'Home'
                      ? tabs[0]
                      : e.key === 'End'
                        ? tabs[tabs.length - 1]
                        : undefined;
              if (next) {
                e.preventDefault();
                setTab(next);
                setChosen(null);
                document.getElementById(`tab-${next}`)?.focus();
              }
            }}
            aria-controls="artifact-panel"
            id={`tab-${t}`}
            onClick={() => {
              setTab(t);
              setChosen(null);
            }}
          >
            {t === 'summary' ? '会议纪要' : t === 'transcribe' ? '逐字稿' : '处理版本'}
          </button>
        ))}
      </div>
      <div
        id="artifact-panel"
        role="tabpanel"
        aria-labelledby={`tab-${tab}`}
        className="document-content"
      >
        {versions.error && (
          <ErrorNotice error={versions.error} retry={() => void versions.refetch()} />
        )}
        {tab === 'versions' ? (
          <>
            <p className="muted">
              显示真实处理次数、产物校验和与来源。失败的新版本不会覆盖旧的已提交产物。
            </p>
            {versions.isPending && <Loading />}
            {available.map((v) => (
              <div className="version-row" key={`${v.artifact.taskId}-${v.artifact.stage}`}>
                <div>
                  <strong>
                    第 {v.attempt} 次 · {v.artifact.stage === 'summary' ? '纪要' : '逐字稿'}
                  </strong>{' '}
                  · {v.current ? '当前' : '历史'} · {ARTIFACT_STATUS[v.status]} /{' '}
                  {STATUS[v.taskStatus]}
                  <p>
                    {dateTime(v.createdAtMs)} · 模型 {v.modelVersion}
                  </p>
                  <details>
                    <summary>校验与输入血缘</summary>
                    <dl>
                      <dt>产物 SHA-256</dt>
                      <dd>{v.sha256 ?? '尚未提交'}</dd>
                      <dt>配置 SHA-256</dt>
                      <dd>{v.configHash}</dd>
                      <dt>来源逐字稿</dt>
                      <dd>
                        {v.sourceTranscriptTaskId ?? '无'} / {v.sourceTranscriptHash ?? '无'}
                      </dd>
                    </dl>
                  </details>
                </div>
                <button
                  disabled={!['committed', 'historical', 'partial'].includes(v.status)}
                  onClick={() => {
                    setChosen(v);
                    setTab(v.artifact.stage);
                  }}
                >
                  读取此版本
                </button>
              </div>
            ))}
            {!versions.isPending && available.length === 0 && (
              <p className="empty-state">尚无处理版本。</p>
            )}
            {versions.hasNextPage && (
              <button
                disabled={versions.isFetchingNextPage}
                onClick={() => void versions.fetchNextPage()}
              >
                更多版本
              </button>
            )}
          </>
        ) : (
          <>
            {selected && (
              <div className="notice">
                第 {selected.attempt} 次处理 · {selected.current ? '当前产物' : '历史产物'} ·{' '}
                {ARTIFACT_STATUS[selected.status]}
                {!selected.current && '；重新处理仍保留此版本'}
              </div>
            )}
            {artifact ? (
              <ArtifactBody
                key={`${artifact.taskId}-${artifact.stage}`}
                artifact={artifact}
                duration={meeting.durationMs}
                locate={locate}
              />
            ) : (
              <p className="empty-state">
                {tab === 'transcribe' && partial
                  ? '正在准备部分逐字稿；可在处理版本中查看已完成片段。'
                  : '尚无对应的有效产物。处理失败时可以到处理版本中查阅旧结果。'}
              </p>
            )}
          </>
        )}
      </div>
    </section>
  );
}
