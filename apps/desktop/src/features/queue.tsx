import { Alert, Button, NativeSelect, Paper, Text, Title } from '@mantine/core';
import { useState } from 'react';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useAction, useAdmission, useBridge } from '../app/context';
import { dateTime } from '../app/dates';
import { ErrorNotice, Heading, Loading, Modal, Status, STATUS } from '../components/ui';
import type { TaskRow, TaskStatus } from '../contracts.generated';
export function TaskControls({ task }: { task: TaskRow }) {
  const bridge = useBridge();
  const admission = useAdmission();
  const action = useAction();
  const [cancel, setCancel] = useState(false);
  const mutate = (command: 'processing_retry' | 'processing_cancel' | 'processing_requeue') =>
    action.run(async () => {
      const result = await bridge.call(command, {
        admission: await admission.obtain(
          command === 'processing_cancel' ? 'cancellation_v1' : 'processing_v1',
        ),
        taskId: task.taskId,
      });
      setCancel(false);
      return result.status === 'completed'
        ? '任务已经完成，产物保留'
        : result.cancelRequested
          ? '取消意图已持久保存；正在等待已拥有的工作进程退出'
          : `任务状态：${STATUS[result.status]}`;
    });
  return (
    <div className="button-row">
      <Button
        disabled={!task.canRetry || action.busy}
        onClick={() => void mutate('processing_retry')}
        type="button"
        variant="default"
      >
        重试失败阶段
      </Button>
      <Button
        disabled={!task.canRequeue || action.busy}
        onClick={() => void mutate('processing_requeue')}
        type="button"
        variant="default"
      >
        重新入队
      </Button>
      <Button
        disabled={!task.canCancel || action.busy}
        onClick={() => {
          action.clearError();
          setCancel(true);
        }}
        type="button"
        variant="default"
      >
        取消任务
      </Button>
      {!!action.error && !cancel && <ErrorNotice error={action.error} />}
      {cancel && (
        <Modal title="取消处理任务" close={() => setCancel(false)} busy={action.busy}>
          <Text component="p">
            保留已经提交的产物。运行中的任务需等待已拥有的子进程退出后才会完成取消；关闭此界面不会撤回取消意图。
          </Text>
          {!!action.error && <ErrorNotice error={action.error} />}
          <div className="modal-actions">
            <Button
              disabled={action.busy}
              onClick={() => setCancel(false)}
              type="button"
              variant="default"
            >
              继续处理
            </Button>
            <Button
              disabled={action.busy}
              onClick={() => void mutate('processing_cancel')}
              type="button"
              variant="default"
            >
              确认取消
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
export function TaskDialog({
  id,
  close,
  openMeeting,
}: {
  id: string;
  close: () => void;
  openMeeting: (id: string) => void;
}) {
  const bridge = useBridge();
  const detail = useQuery({
    queryKey: ['task_get', id],
    queryFn: ({ signal }) => bridge.call('task_get', { taskId: id }, signal),
    refetchInterval: 2000,
  });
  return (
    <Modal title="任务详情" close={close}>
      {detail.isPending ? (
        <Loading />
      ) : detail.error ? (
        <ErrorNotice error={detail.error} retry={() => void detail.refetch()} />
      ) : (
        <>
          <Text component="p">
            第 {detail.data.task.attempt} 次 · <Status status={detail.data.task.status} />
          </Text>
          <Text component="p">
            {detail.data.task.completedSegments} / {detail.data.task.totalSegments} 段 · 已重试{' '}
            {detail.data.task.retryCount} 次
          </Text>
          <Text component="p">下次重试：{dateTime(detail.data.task.nextRetryAtMs)}</Text>
          {detail.data.task.cancelRequested && (
            <Alert className="notice" variant="light" role="status">
              取消请求已持久保存，等待工作进程收束。
            </Alert>
          )}
          {detail.data.task.error && (
            <ErrorNotice error={new Error(detail.data.task.error.message)} />
          )}
          <TaskControls task={detail.data.task} />
          <Title order={3}>脱敏诊断</Title>
          {detail.data.diagnostics.map((d, i) => (
            <Text key={i} component="p">
              {d}
            </Text>
          ))}
          <Button
            onClick={() => {
              openMeeting(detail.data.task.meetingId);
              close();
            }}
            type="button"
            variant="default"
          >
            查看会议及旧产物
          </Button>
        </>
      )}
    </Modal>
  );
}
export function Queue({ openMeeting }: { openMeeting: (id: string) => void }) {
  const bridge = useBridge();
  const [status, setStatus] = useState<TaskStatus | ''>('');
  const [selected, setSelected] = useState<string | null>(null);
  const tasks = useInfiniteQuery({
    queryKey: ['task_list', status],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) =>
      bridge.call(
        'task_list',
        { meetingId: null, statuses: status ? [status] : [], cursor: pageParam, pageSize: 25 },
        signal,
      ),
    getNextPageParam: (p) => p.nextCursor ?? undefined,
    refetchInterval: 2000,
  });
  return (
    <>
      <Heading
        title="处理队列"
        text="后台任务独立于窗口；关闭 App 不会取消任务。重试沿用有限重试和有效检查点。"
      />
      <Paper className="panel" withBorder p="md" component="section">
        <div className="filter-top">
          <label>
            任务状态
            <NativeSelect
              value={status}
              onChange={(e) => setStatus(e.target.value as typeof status)}
            >
              <option value="">全部状态</option>
              {Object.entries(STATUS)
                .filter(([k]) => !['recording', 'unsealed', 'empty', 'unknown'].includes(k))
                .map(([k, v]) => (
                  <option key={k} value={k}>
                    {v}
                  </option>
                ))}
            </NativeSelect>
          </label>
        </div>
        {tasks.isPending ? (
          <Loading />
        ) : tasks.error ? (
          <ErrorNotice error={tasks.error} retry={() => void tasks.refetch()} />
        ) : (
          <>
            {tasks.data.pages
              .flatMap((p) => p.items)
              .map((task) => (
                <article className="task-row" key={task.taskId}>
                  <div>
                    <Title order={3}>
                      <Button
                        className="text-button"
                        onClick={() => setSelected(task.taskId)}
                        type="button"
                        variant="subtle"
                      >
                        第 {task.attempt} 次 · {task.stage === 'transcribe' ? '转写' : '纪要'}
                      </Button>
                    </Title>
                    <Text className="muted" component="p" c="dimmed">
                      {task.meetingId} · {task.completedSegments} / {task.totalSegments} 段
                    </Text>
                    <Text component="p">
                      <Status status={task.status} />
                      {task.cancelRequested && ' · 正在取消'} · 重试 {task.retryCount} 次
                    </Text>
                    {task.error && (
                      <Text className="error-text" component="p" c="red">
                        {task.error.message}
                      </Text>
                    )}
                  </div>
                  <TaskControls task={task} />
                  <Button
                    onClick={() => openMeeting(task.meetingId)}
                    type="button"
                    variant="default"
                  >
                    查看会议
                  </Button>
                </article>
              ))}
            {tasks.data.pages.every((p) => p.items.length === 0) && (
              <Text className="empty-state" component="p">
                没有符合条件的任务。
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
          </>
        )}
      </Paper>
      {selected && (
        <TaskDialog id={selected} close={() => setSelected(null)} openMeeting={openMeeting} />
      )}
    </>
  );
}
