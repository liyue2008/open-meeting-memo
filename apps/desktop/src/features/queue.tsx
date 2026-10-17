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
      <button
        disabled={!task.canRetry || action.busy}
        onClick={() => void mutate('processing_retry')}
      >
        重试失败阶段
      </button>
      <button
        disabled={!task.canRequeue || action.busy}
        onClick={() => void mutate('processing_requeue')}
      >
        重新入队
      </button>
      <button disabled={!task.canCancel || action.busy} onClick={() => setCancel(true)}>
        取消任务
      </button>
      {!!action.error && <ErrorNotice error={action.error} />}
      {cancel && (
        <Modal title="取消处理任务" close={() => setCancel(false)} busy={action.busy}>
          <p>
            保留已经提交的产物。运行中的任务需等待已拥有的子进程退出后才会完成取消；关闭此界面不会撤回取消意图。
          </p>
          <div className="modal-actions">
            <button disabled={action.busy} onClick={() => setCancel(false)}>
              继续处理
            </button>
            <button disabled={action.busy} onClick={() => void mutate('processing_cancel')}>
              确认取消
            </button>
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
          <p>
            第 {detail.data.task.attempt} 次 · <Status status={detail.data.task.status} />
          </p>
          <p>
            {detail.data.task.completedSegments} / {detail.data.task.totalSegments} 段 · 已重试{' '}
            {detail.data.task.retryCount} 次
          </p>
          <p>下次重试：{dateTime(detail.data.task.nextRetryAtMs)}</p>
          {detail.data.task.cancelRequested && (
            <div className="notice">取消请求已持久保存，等待工作进程收束。</div>
          )}
          {detail.data.task.error && (
            <ErrorNotice error={new Error(detail.data.task.error.message)} />
          )}
          <TaskControls task={detail.data.task} />
          <h3>脱敏诊断</h3>
          {detail.data.diagnostics.map((d, i) => (
            <p key={i}>{d}</p>
          ))}
          <button
            onClick={() => {
              openMeeting(detail.data.task.meetingId);
              close();
            }}
          >
            查看会议及旧产物
          </button>
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
      <section className="panel">
        <div className="filter-top">
          <label>
            任务状态
            <select value={status} onChange={(e) => setStatus(e.target.value as typeof status)}>
              <option value="">全部状态</option>
              {Object.entries(STATUS)
                .filter(([k]) => !['recording', 'unsealed', 'empty', 'unknown'].includes(k))
                .map(([k, v]) => (
                  <option key={k} value={k}>
                    {v}
                  </option>
                ))}
            </select>
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
                    <h3>
                      <button className="text-button" onClick={() => setSelected(task.taskId)}>
                        第 {task.attempt} 次 · {task.stage === 'transcribe' ? '转写' : '纪要'}
                      </button>
                    </h3>
                    <p className="muted">
                      {task.meetingId} · {task.completedSegments} / {task.totalSegments} 段
                    </p>
                    <p>
                      <Status status={task.status} />
                      {task.cancelRequested && ' · 正在取消'} · 重试 {task.retryCount} 次
                    </p>
                    {task.error && <p className="error-text">{task.error.message}</p>}
                  </div>
                  <TaskControls task={task} />
                  <button onClick={() => openMeeting(task.meetingId)}>查看会议</button>
                </article>
              ))}
            {tasks.data.pages.every((p) => p.items.length === 0) && (
              <p className="empty-state">没有符合条件的任务。</p>
            )}
            {tasks.hasNextPage && (
              <button
                disabled={tasks.isFetchingNextPage}
                onClick={() => void tasks.fetchNextPage()}
              >
                更多任务
              </button>
            )}
          </>
        )}
      </section>
      {selected && (
        <TaskDialog id={selected} close={() => setSelected(null)} openMeeting={openMeeting} />
      )}
    </>
  );
}
