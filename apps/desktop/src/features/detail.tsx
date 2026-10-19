import { Alert, Badge, Button, Paper, Text, Title } from '@mantine/core';
import { useState, useSyncExternalStore } from 'react';
import { focusManager, useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useBridge } from '../app/context';
import { clock, dateTime } from '../app/dates';
import { ErrorNotice, Heading, Loading, SOURCES, Status } from '../components/ui';
import { EditMeeting, Reprocess, RetainMeetings } from './meeting-actions';
import { Player } from './player';
import { Artifacts } from './artifacts';
import { isActiveMeetingStatus } from './meeting-task-activity';
import { TaskControls, TaskDialog } from './queue';
const subscribeQueryFocus = (notify: () => void) => focusManager.subscribe(notify);
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
  const queryFocused = useSyncExternalStore(subscribeQueryFocus, () => focusManager.isFocused());
  const [dialog, setDialog] = useState<'edit' | 'trash' | 'process' | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [seek, setSeek] = useState<{ time: number; serial: number } | null>(null);
  const [transcript, setTranscript] = useState<string | null>(null);
  const detail = useQuery({
    queryKey: ['meeting_get', id],
    queryFn: ({ signal }) => bridge.call('meeting_get', { meetingId: id }, signal),
    refetchInterval: (query) => {
      if (isActiveMeetingStatus(query.state.data?.meeting.status)) return 2000;
      // Keep authority current while hidden; terminal details retain ordinary foreground refresh.
      return queryFocused ? 10000 : false;
    },
    refetchIntervalInBackground: true,
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
        <Button onClick={back} type="button" variant="default">
          ← 会议库
        </Button>
        <ErrorNotice error={detail.error} retry={() => void detail.refetch()} />
      </>
    );
  const data = detail.data;
  const meeting = data.meeting;
  return (
    <>
      <Button className="back-link text-button" onClick={back} type="button" variant="subtle">
        ← 返回会议库
      </Button>
      <Heading
        title={meeting.metadata.title}
        text={`${dateTime(meeting.startedAtMs)} · ${SOURCES[meeting.source]} · ${meeting.durationMs === null ? '时长未知' : clock(meeting.durationMs)}`}
      >
        <Status status={meeting.status} />
        <Button
          disabled={!meeting.capabilities.edit}
          onClick={() => setDialog('edit')}
          type="button"
          variant="default"
        >
          编辑
        </Button>
        <Button
          disabled={!meeting.capabilities.reprocess}
          onClick={() => setDialog('process')}
          type="button"
          variant="default"
        >
          重新处理
        </Button>
        <Button
          disabled={meeting.trashed ? !meeting.capabilities.restore : !meeting.capabilities.trash}
          onClick={() => setDialog('trash')}
          type="button"
          variant="default"
        >
          {meeting.trashed ? '恢复会议' : '移入回收站'}
        </Button>
      </Heading>
      {meeting.trashed && (
        <Alert className="notice" variant="light" role="status">
          此会议位于回收站，录音和产物保留。
        </Alert>
      )}
      <div className="tag-list">
        {meeting.metadata.tags.map((t) => (
          <Badge className="tag" key={t}>
            {t}
          </Badge>
        ))}
      </div>
      {meeting.metadata.note && (
        <Text className="meeting-note" component="p">
          {meeting.metadata.note}
        </Text>
      )}
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
      <Paper className="panel section-spaced" withBorder p="md" component="section">
        <Title order={2}>本场处理任务</Title>
        {tasks.isPending && <Loading />}
        {tasks.error && <ErrorNotice error={tasks.error} retry={() => void tasks.refetch()} />}{' '}
        {tasks.data?.pages
          .flatMap((p) => p.items)
          .map((t) => (
            <article className="task-row" key={t.taskId}>
              <div>
                <Button
                  className="text-button"
                  onClick={() => setTaskId(t.taskId)}
                  type="button"
                  variant="subtle"
                >
                  第 {t.attempt} 次 · {t.stage === 'summary' ? '纪要' : '转写'}
                </Button>
                <Text component="p">
                  <Status status={t.status} /> · {t.completedSegments} / {t.totalSegments} 段
                  {t.cancelRequested && ' · 取消处理中'}
                </Text>
              </div>
              <TaskControls task={t} />
            </article>
          ))}
        {tasks.data?.pages.every((p) => p.items.length === 0) && (
          <Text className="empty-state" component="p">
            尚无处理任务。
          </Text>
        )}
        {tasks.hasNextPage && (
          <Button
            disabled={tasks.isFetchingNextPage}
            onClick={() => void tasks.fetchNextPage()}
            type="button"
            variant="default"
          >
            更多任务
          </Button>
        )}
      </Paper>
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
