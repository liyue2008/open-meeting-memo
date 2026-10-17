import { useState } from 'react';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useBridge } from '../app/context';
import { clock, dateTime } from '../app/dates';
import { ErrorNotice, Heading, Loading, SOURCES, Status } from '../components/ui';
import { EditMeeting, Reprocess, RetainMeetings } from './meeting-actions';
import { Player } from './player';
import { Artifacts } from './artifacts';
import { TaskControls, TaskDialog } from './queue';
export function Detail({
  id,
  back,
  openMeeting,
}: {
  id: string;
  back: () => void;
  openMeeting: (id: string) => void;
}) {
  const bridge = useBridge();
  const [dialog, setDialog] = useState<'edit' | 'trash' | 'process' | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [seek, setSeek] = useState<{ time: number; serial: number } | null>(null);
  const [transcript, setTranscript] = useState<string | null>(null);
  const detail = useQuery({
    queryKey: ['meeting_get', id],
    queryFn: ({ signal }) => bridge.call('meeting_get', { meetingId: id }, signal),
    refetchInterval: 10000,
  });
  const tasks = useInfiniteQuery({
    queryKey: ['task_list', id],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) =>
      bridge.call(
        'task_list',
        { meetingId: id, statuses: [], cursor: pageParam, pageSize: 25 },
        signal,
      ),
    getNextPageParam: (p) => p.nextCursor ?? undefined,
    refetchInterval: 5000,
  });
  if (detail.isPending) return <Loading />;
  if (detail.error)
    return (
      <>
        <button onClick={back}>← 会议库</button>
        <ErrorNotice error={detail.error} retry={() => void detail.refetch()} />
      </>
    );
  const data = detail.data;
  const meeting = data.meeting;
  return (
    <>
      <button className="back-link text-button" onClick={back}>
        ← 返回会议库
      </button>
      <Heading
        title={meeting.metadata.title}
        text={`${dateTime(meeting.startedAtMs)} · ${SOURCES[meeting.source]} · ${meeting.durationMs === null ? '时长未知' : clock(meeting.durationMs)}`}
      >
        <Status status={meeting.status} />
        <button disabled={!meeting.capabilities.edit} onClick={() => setDialog('edit')}>
          编辑
        </button>
        <button disabled={!meeting.capabilities.reprocess} onClick={() => setDialog('process')}>
          重新处理
        </button>
        <button
          disabled={meeting.trashed ? !meeting.capabilities.restore : !meeting.capabilities.trash}
          onClick={() => setDialog('trash')}
        >
          {meeting.trashed ? '恢复会议' : '移入回收站'}
        </button>
      </Heading>
      {meeting.trashed && <div className="notice">此会议位于回收站，录音和产物保留。</div>}
      <div className="tag-list">
        {meeting.metadata.tags.map((t) => (
          <span className="tag" key={t}>
            {t}
          </span>
        ))}
      </div>
      {meeting.metadata.note && <p className="meeting-note">{meeting.metadata.note}</p>}
      <Player meetingId={id} seek={seek} />
      <Artifacts
        meeting={meeting}
        partial={data.hasPartialTranscript}
        locate={(time) => {
          setSeek((s) => ({ time, serial: (s?.serial ?? 0) + 1 }));
          document
            .querySelector('.audio-player')
            ?.scrollIntoView({ behavior: 'smooth', block: 'start' });
        }}
        onTranscript={setTranscript}
      />
      <section className="panel section-spaced">
        <h2>本场处理任务</h2>
        {tasks.isPending && <Loading />}
        {tasks.error && <ErrorNotice error={tasks.error} retry={() => void tasks.refetch()} />}{' '}
        {tasks.data?.pages
          .flatMap((p) => p.items)
          .map((t) => (
            <article className="task-row" key={t.taskId}>
              <div>
                <button className="text-button" onClick={() => setTaskId(t.taskId)}>
                  第 {t.attempt} 次 · {t.stage === 'summary' ? '纪要' : '转写'}
                </button>
                <p>
                  <Status status={t.status} /> · {t.completedSegments} / {t.totalSegments} 段
                  {t.cancelRequested && ' · 取消处理中'}
                </p>
              </div>
              <TaskControls task={t} />
            </article>
          ))}
        {tasks.data?.pages.every((p) => p.items.length === 0) && (
          <p className="empty-state">尚无处理任务。</p>
        )}
        {tasks.hasNextPage && (
          <button disabled={tasks.isFetchingNextPage} onClick={() => void tasks.fetchNextPage()}>
            更多任务
          </button>
        )}
      </section>
      {dialog === 'edit' && <EditMeeting meeting={meeting} close={() => setDialog(null)} />}{' '}
      {dialog === 'trash' && (
        <RetainMeetings
          meetings={[meeting]}
          restore={meeting.trashed}
          close={() => setDialog(null)}
        />
      )}{' '}
      {dialog === 'process' && (
        <Reprocess
          meetings={[meeting]}
          sourceTranscriptTaskId={transcript}
          close={() => setDialog(null)}
        />
      )}{' '}
      {taskId && <TaskDialog id={taskId} close={() => setTaskId(null)} openMeeting={openMeeting} />}
    </>
  );
}
