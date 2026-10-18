import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useAction, useAdmission, useBridge } from '../app/context';
import { dateTime, localTimestamp } from '../app/dates';
import { ErrorNotice, Modal } from '../components/ui';
import type { ImportSelection, MeetingRow, ProcessingStage } from '../contracts.generated';

export function EditMeeting({ meeting, close }: { meeting: MeetingRow; close: () => void }) {
  const bridge = useBridge();
  const admission = useAdmission();
  const action = useAction();
  const [title, setTitle] = useState(meeting.metadata.title);
  const [note, setNote] = useState(meeting.metadata.note);
  const [tags, setTags] = useState(meeting.metadata.tags.join(', '));
  return (
    <Modal title="编辑会议" close={close} busy={action.busy}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void action.run(async () => {
            await bridge.call('meeting_update', {
              admission: await admission.obtain('metadata_v1'),
              meetingId: meeting.meetingId,
              expectedRevision: meeting.revision,
              metadata: {
                title,
                note,
                tags: tags
                  .split(/[,，]/)
                  .map((s) => s.trim())
                  .filter(Boolean),
              },
            });
            close();
            return '会议资料已保存';
          });
        }}
      >
        <label>
          会议标题
          <input
            autoFocus
            required
            maxLength={512}
            value={title}
            onChange={(e) => setTitle(Array.from(e.target.value).slice(0, 256).join(''))}
          />
        </label>
        <label>
          备注
          <textarea value={note} onChange={(e) => setNote(e.target.value)} />
        </label>
        <label>
          标签（逗号分隔）
          <input value={tags} onChange={(e) => setTags(e.target.value)} />
        </label>
        <p className="muted">
          只更新标题、备注和标签；不改写录音或原始产物。版本冲突时保留草稿，请关闭后重新读取再合并。
        </p>
        {!!action.error && <ErrorNotice error={action.error} />}
        <div className="modal-actions">
          <button type="button" onClick={close} disabled={action.busy}>
            放弃
          </button>
          <button className="primary" disabled={action.busy}>
            保存
          </button>
        </div>
      </form>
    </Modal>
  );
}
export function RetainMeetings({
  meetings,
  restore,
  close,
}: {
  meetings: MeetingRow[];
  restore: boolean;
  close: () => void;
}) {
  const bridge = useBridge();
  const admission = useAdmission();
  const action = useAction();
  return (
    <Modal title={restore ? '恢复会议' : '移入回收站'} close={close} busy={action.busy}>
      <p>
        {restore
          ? '恢复后会议将重新出现在会议库。'
          : '录音、产物和处理历史会完整保留。本版本不提供永久删除。活动或未封存会议不能删除。'}
      </p>
      <p>共 {meetings.length} 场会议；批量操作全部成功才提交。</p>
      {!!action.error && <ErrorNotice error={action.error} />}
      <div className="modal-actions">
        <button disabled={action.busy} onClick={close}>
          返回
        </button>
        <button
          className={restore ? 'primary' : 'danger'}
          disabled={action.busy}
          onClick={() =>
            void action.run(async () => {
              await bridge.call(restore ? 'meeting_restore' : 'meeting_trash', {
                admission: await admission.obtain('trash_v1'),
                meetings: meetings.map((m) => ({
                  meetingId: m.meetingId,
                  expectedRevision: m.revision,
                })),
              });
              close();
              return restore ? '会议已恢复' : '已移入回收站，原始内容保留';
            })
          }
        >
          {restore ? '确认恢复' : '移入回收站'}
        </button>
      </div>
    </Modal>
  );
}
export function Reprocess({
  meetings,
  close,
  sourceTranscriptTaskId = null,
}: {
  meetings: MeetingRow[];
  close: () => void;
  sourceTranscriptTaskId?: string | null;
}) {
  const bridge = useBridge();
  const admission = useAdmission();
  const action = useAction();
  const [stage, setStage] = useState<ProcessingStage>('all');
  const [outcomes, setOutcomes] = useState<
    Array<{
      meetingId: string;
      error: string | null;
      taskId: string | null;
      operationId: string | null;
    }>
  >([]);
  return (
    <Modal title="重新处理" close={close} busy={action.busy}>
      <p>
        创建新处理版本，保留旧录音、逐字稿和纪要。新的转写成功后会继续生成纪要；界面等待结束不会取消后台任务。
      </p>
      <label>
        处理范围
        <select value={stage} onChange={(e) => setStage(e.target.value as ProcessingStage)}>
          <option value="all">重新转写并生成纪要</option>
          <option value="transcribe">重新转写</option>
          <option value="summary">仅重新生成纪要</option>
        </select>
      </label>
      {stage === 'summary' && (
        <p className="muted">
          {sourceTranscriptTaskId
            ? '复用所选的已提交逐字稿（当前或历史版本）。'
            : '复用每场会议当前有效的已提交逐字稿。'}
        </p>
      )}
      {outcomes.map((o) => (
        <p key={o.meetingId}>
          {o.meetingId}：
          {o.error ??
            (o.taskId
              ? `任务已入队 ${o.taskId}`
              : o.operationId
                ? `后台操作 ${o.operationId}`
                : '服务未返回任务标识，请刷新队列确认')}
        </p>
      ))}
      {!!action.error && <ErrorNotice error={action.error} />}
      <div className="modal-actions">
        <button disabled={action.busy} onClick={close}>
          关闭
        </button>
        <button
          className="primary"
          disabled={action.busy || outcomes.length > 0}
          onClick={() =>
            void action.run(async () => {
              const result = await bridge.call('processing_enqueue', {
                admission: await admission.obtain('processing_v1'),
                meetingIds: meetings.map((m) => m.meetingId),
                stage,
                sourceTranscriptTaskId:
                  stage === 'summary' && meetings.length === 1 ? sourceTranscriptTaskId : null,
              });
              setOutcomes(
                result.items.map((o) => ({
                  meetingId: o.meetingId,
                  taskId: o.taskId,
                  operationId: o.operationId,
                  error: o.error?.message ?? null,
                })),
              );
              return '请查看每项结果及处理队列';
            })
          }
        >
          开始处理 {meetings.length} 场
        </button>
      </div>
    </Modal>
  );
}
export function CreateMeeting({ close }: { close: () => void }) {
  const bridge = useBridge();
  const admission = useAdmission();
  const action = useAction();
  const [selection, setSelection] = useState<ImportSelection | null>(null);
  const selected = useRef<ImportSelection | null>(null);
  const committed = useRef(false);
  const [title, setTitle] = useState('');
  const [consent, setConsent] = useState(false);
  const [started, setStarted] = useState('');
  useEffect(
    () => () => {
      if (selected.current && !committed.current)
        void bridge
          .call('import_cancel', { selectionHandle: selected.current.selectionHandle })
          .catch(() => undefined);
    },
    [bridge],
  );
  return (
    <Modal title="新建会议" close={close} busy={action.busy}>
      <p>手动开始录音，或导入一份已经封存的音频。不会创建没有来源的空会议。</p>
      <label>
        会议标题
        <input
          autoFocus
          value={title}
          required
          maxLength={512}
          onChange={(e) => setTitle(Array.from(e.target.value).slice(0, 256).join(''))}
        />
      </label>
      <div className="panel section-spaced">
        <h3>导入已有录音</h3>
        <p>WAV 或 Ogg/Opus，最多 2 GiB / 8 小时。原文件保留，导入内容标记为混音。</p>
        <button
          disabled={action.busy}
          onClick={() =>
            void action.run(async () => {
              if (selected.current)
                await bridge.call('import_cancel', {
                  selectionHandle: selected.current.selectionHandle,
                });
              selected.current = null;
              setSelection(null);
              const result = await bridge.call('import_select', {});
              selected.current = result.selection;
              setSelection(result.selection);
              if (!result.selection) return '已取消文件选择';
            })
          }
        >
          选择音频文件
        </button>
        {selection && (
          <>
            <p>
              {selection.displayName} · {Math.round(selection.durationMs / 1000)} 秒 ·{' '}
              {selection.channels} 声道
            </p>
            <label>
              原会议开始时间（本地）
              <input
                type="datetime-local"
                value={started}
                onChange={(e) => setStarted(e.target.value)}
              />
            </label>
            <button
              disabled={action.busy || !title.trim() || !started}
              onClick={() =>
                void action.run(async () => {
                  const start = localTimestamp(started);
                  const result = await bridge.call('import_commit', {
                    admission: await admission.obtain('import_mix_v1'),
                    selectionHandle: selection.selectionHandle,
                    metadata: { title, note: '', tags: [] },
                    startedAtMs: start,
                    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
                  });
                  committed.current = true;
                  close();
                  return result.duplicate ? '相同音频已存在，未重复导入' : '导入已提交';
                })
              }
            >
              导入
            </button>
          </>
        )}
      </div>
      <div className="panel section-spaced">
        <h3>手动录音</h3>
        <label>
          <input type="checkbox" checked={consent} onChange={(e) => setConsent(e.target.checked)} />{' '}
          我确认参与者同意录音，并了解系统音频可能包含其他应用声音。
        </label>
        <button
          className="primary"
          disabled={action.busy || !consent || !title.trim() || !!selection}
          onClick={() =>
            void action.run(async () => {
              const result = await bridge.call('record_start', {
                admission: await admission.obtain(),
                title,
                confirmRecordingConsent: consent,
              });
              if (result.error) throw new Error(result.error.message);
              close();
              return '录音状态已更新，请查看状态栏';
            })
          }
        >
          开始录音
        </button>
      </div>
      {!!action.error && <ErrorNotice error={action.error} />}
    </Modal>
  );
}
export function RecordingBar() {
  const bridge = useBridge();
  const admission = useAdmission();
  const action = useAction();
  const [confirmAuto, setConfirmAuto] = useState(false);
  const status = useQuery({
    queryKey: ['record_status'],
    queryFn: ({ signal }) => bridge.call('record_status', {}, signal),
    refetchInterval: 10000,
  });
  if (status.error)
    return (
      <div className="notice" role="status">
        录音状态与已保存的自动录音偏好未知，不能更改。
        <ErrorNotice error={status.error} retry={() => void status.refetch()} />
      </div>
    );
  if (!status.data) return <div role="status">正在读取录音状态；已保存的自动录音偏好未知。</div>;
  const data = status.data;
  const autoBlocked =
    data.autoRecord === null || data.recordingConsent === null || !data.compatibility.writesAllowed;
  return (
    <div className="recording-bar">
      <span>
        {data.state === 'recording'
          ? `正在录音 · ${dateTime(data.startedAtMs)}`
          : data.state === 'sealing'
            ? '正在封存，请等待'
            : data.state === 'unknown'
              ? '录音状态未知'
              : '当前未录音'}{' '}
        · 已保存的自动录音偏好：
        {data.autoRecord === null ? '未知' : data.autoRecord ? '已启用' : '已关闭'} · 录音同意：
        {data.recordingConsent === null ? '未知' : data.recordingConsent ? '已确认' : '未确认'}
      </span>
      <div className="button-row">
        {data.state === 'recording' && (
          <button
            disabled={action.busy}
            onClick={() =>
              void action.run(async () => {
                const result = await bridge.call('record_stop', {
                  admission: await admission.obtain(),
                });
                if (result.error) throw new Error(result.error.message);
                return '停止请求已返回，请确认封存状态';
              })
            }
          >
            停止并封存
          </button>
        )}
        <button
          disabled={action.busy || autoBlocked}
          onClick={() => {
            if (data.autoRecord)
              void action.run(async () => {
                const result = await bridge.call('record_auto_set', {
                  admission: await admission.obtain(),
                  enabled: false,
                  confirmRecordingConsent: false,
                });
                if (result.error) throw new Error(result.error.message);
                if (result.autoRecord !== false) throw new Error('自动录音尚未关闭，请查看诊断。');
                return '自动录音已关闭';
              });
            else setConfirmAuto(true);
          }}
        >
          {data.autoRecord === null
            ? '自动录音偏好未知'
            : data.autoRecord
              ? '关闭自动录音'
              : '启用自动录音'}
        </button>
      </div>
      {(data.autoRecord === null ||
        data.recordingConsent === null ||
        !data.compatibility.writesAllowed) && (
        <p role="status">
          当前不能更改：
          {data.autoRecord === null || data.recordingConsent === null
            ? '已保存的偏好或同意尚未确认，请重新读取。'
            : '当前服务未授权写入，保留既有偏好。'}
        </p>
      )}
      {!!action.error && !confirmAuto && <ErrorNotice error={action.error} />}
      {confirmAuto && (
        <Modal title="启用自动录音" close={() => setConfirmAuto(false)} busy={action.busy}>
          {(autoBlocked || data.autoRecord !== false) && (
            <p role="status">不能确认：偏好、同意或写入授权已变化或未知，请关闭后重新读取。</p>
          )}
          <p>
            只有飞书强信号确认参会后才会自动录音。请确认所有参与者同意；系统声音可能包含其他应用音频。
          </p>
          <div className="modal-actions">
            <button disabled={action.busy} onClick={() => setConfirmAuto(false)}>
              返回
            </button>
            <button
              disabled={action.busy || autoBlocked || data.autoRecord !== false}
              onClick={() =>
                void action.run(async () => {
                  const result = await bridge.call('record_auto_set', {
                    admission: await admission.obtain(),
                    enabled: true,
                    confirmRecordingConsent: true,
                  });
                  if (result.error) throw new Error(result.error.message);
                  if (result.autoRecord !== true) throw new Error('自动录音尚未启用，请查看诊断。');
                  setConfirmAuto(false);
                  return '自动录音已启用';
                })
              }
            >
              确认同意并启用
            </button>
          </div>
          {!!action.error && <ErrorNotice error={action.error} />}
        </Modal>
      )}
    </div>
  );
}
