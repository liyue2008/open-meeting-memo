import { Alert, Button, Switch, Fieldset, Paper, Text, TextInput, Textarea } from '@mantine/core';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useAction, useAdmission, useBridge } from '../app/context';
import { Disclosure, ErrorNotice, Heading, Loading, Modal } from '../components/ui';
import type {
  ConfigPatch,
  ConfigSnapshot,
  ConfigValidation,
  EditableConfig,
} from '../contracts.generated';
export interface DraftGuard {
  dirty: boolean;
  save: () => Promise<boolean>;
  discard: () => void;
}
export type RegisterGuard = (guard: DraftGuard | null) => void;
type Field = {
  label: string;
  group: string;
  type: 'number' | 'text' | 'boolean' | 'lines' | 'events' | 'nullable';
  help?: string;
};
const fields: Record<keyof EditableConfig, Field> = {
  opusBitrateKbps: {
    label: 'Opus 比特率（kbps）',
    group: '录音',
    type: 'number',
    help: '支持 32 / 48 / 64 / 96，新分段生效。',
  },
  notificationsEnabled: { label: '启用桌面通知', group: '通知', type: 'boolean' },
  notificationsShowMeetingTitle: { label: '通知显示会议标题', group: '通知', type: 'boolean' },
  notificationEvents: {
    label: '通知事件（每行一个）',
    group: '通知',
    type: 'events',
    help: 'recording_started / recording_stopped / summary_ready / recording_failed / asr_failed / summary_failed / test；保存不发送测试通知。',
  },
  captureExecutable: { label: '录音辅助程序', group: '录音', type: 'text' },
  workerExecutable: { label: '转写工作进程', group: '本地转写', type: 'text' },
  larkExecutable: { label: '飞书可执行程序', group: '会议检测', type: 'text' },
  rtcLogDirs: { label: '飞书 RTC 日志目录（每行一个）', group: '会议检测', type: 'lines' },
  startDelayMs: { label: '自动录音开始延迟（毫秒）', group: '会议检测', type: 'number' },
  endGraceMs: { label: '会议结束宽限（毫秒）', group: '会议检测', type: 'number' },
  pollMs: { label: '检测间隔（毫秒）', group: '会议检测', type: 'number' },
  segmentMs: { label: '录音分段时长（毫秒）', group: '录音', type: 'number' },
  asrLibraryPath: { label: 'sherpa-onnx 动态库', group: '本地转写', type: 'text' },
  asrEncoderPath: { label: '编码器模型', group: '本地转写', type: 'text' },
  asrDecoderPath: { label: '解码器模型', group: '本地转写', type: 'text' },
  asrTokensPath: { label: '词表', group: '本地转写', type: 'text' },
  asrVadPath: { label: '语音检测模型', group: '本地转写', type: 'text' },
  asrThreads: { label: '转写线程数', group: '本地转写', type: 'number' },
  asrTimeoutMs: { label: '转写超时（毫秒）', group: '本地转写', type: 'number' },
  piExecutable: { label: 'Pi 可执行程序', group: '纪要', type: 'text' },
  piNodeExecutable: { label: 'Pi 的 Node 程序（空为未指定）', group: '纪要', type: 'nullable' },
  piProvider: { label: 'Pi 提供商', group: '纪要', type: 'text' },
  piModel: { label: 'Pi 模型', group: '纪要', type: 'text' },
  piPromptVersion: { label: '纪要提示词版本', group: '纪要', type: 'text' },
  piTimeoutMs: { label: 'Pi 超时（毫秒）', group: '纪要', type: 'number' },
  piMaxInputBytes: { label: 'Pi 最大输入（字节）', group: '纪要', type: 'number' },
  piMaxOutputBytes: { label: 'Pi 最大输出（字节）', group: '纪要', type: 'number' },
};
type Draft = Record<keyof EditableConfig, string | boolean>;
export function configDraft(values: EditableConfig): Draft {
  return Object.fromEntries(
    Object.entries(values).map(([k, v]) => [
      k,
      Array.isArray(v) ? v.join('\n') : typeof v === 'boolean' ? v : String(v ?? ''),
    ]),
  ) as Draft;
}
export function configPatch(base: EditableConfig, draft: Draft): ConfigPatch {
  const entries: Array<[string, unknown]> = [];
  for (const key of Object.keys(fields) as Array<keyof EditableConfig>) {
    const descriptor = fields[key];
    const raw = draft[key];
    let value: unknown = raw;
    if (descriptor.type === 'number') {
      if (
        typeof raw !== 'string' ||
        !raw.trim() ||
        !/^\d+$/.test(raw.trim()) ||
        !Number.isSafeInteger(Number(raw))
      )
        throw new Error(`${descriptor.label}必须是非负安全整数`);
      value = Number(raw);
    } else if (descriptor.type === 'lines' || descriptor.type === 'events')
      value = String(raw)
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean);
    else if (descriptor.type === 'nullable') value = raw === '' ? null : raw;
    if (JSON.stringify(value) !== JSON.stringify(base[key])) entries.push([key, value]);
  }
  return Object.fromEntries(entries) as ConfigPatch;
}
const effectName = {
  immediate: '立即',
  new_task: '新任务',
  new_segment: '新录音分段',
  service_reload: '服务重新载入',
  service_restart: '服务重启',
};
export function Settings({ register }: { register: RegisterGuard }) {
  const bridge = useBridge();
  const query = useQuery({
    queryKey: ['config_get'],
    queryFn: ({ signal }) => bridge.call('config_get', {}, signal),
    refetchOnWindowFocus: false,
  });
  return (
    <>
      <Heading
        title="设置"
        text="配置保存在本机；保存前进行原生校验，并检查读取以来是否被其他进程修改。"
      />
      {query.isPending ? (
        <Loading />
      ) : query.error ? (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      ) : (
        <SettingsForm initial={query.data} register={register} />
      )}
    </>
  );
}
function SettingsForm({ initial, register }: { initial: ConfigSnapshot; register: RegisterGuard }) {
  const bridge = useBridge();
  const admission = useAdmission();
  const action = useAction();
  const [base, setBase] = useState(initial);
  const known = base.initialized && base.autoRecord !== null && base.recordingConsent !== null;
  const [draft, setDraft] = useState(() => configDraft(initial.values));
  const [validation, setValidation] = useState<ConfigValidation | null>(null);
  const [validationError, setValidationError] = useState<unknown>(null);
  const [validating, setValidating] = useState(false);
  const [discard, setDiscard] = useState(false);
  let patch: ConfigPatch = {};
  let parseError: unknown = null;
  try {
    patch = configPatch(base.values, draft);
  } catch (e) {
    parseError = e;
  }
  const dirty = JSON.stringify(draft) !== JSON.stringify(configDraft(base.values));
  const patchKey = JSON.stringify(patch);
  const draftRevision = useRef(0);
  useEffect(() => {
    draftRevision.current++;
  }, [draft]);
  useEffect(() => {
    const controller = new AbortController();
    if (parseError) {
      setValidation(null);
      setValidationError(parseError);
      setValidating(false);
      return;
    }
    setValidating(true);
    setValidation(null);
    setValidationError(null);
    const timer = setTimeout(() => {
      void bridge
        .call('config_validate', { patch }, controller.signal)
        .then((r) => {
          if (!controller.signal.aborted) {
            setValidation(r);
            setValidating(false);
          }
        })
        .catch((e) => {
          if (!controller.signal.aborted) {
            setValidationError(e);
            setValidating(false);
          }
        });
    }, 350);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [bridge, patchKey, String(parseError)]);
  const reset = useCallback(() => {
    setDraft(configDraft(base.values));
    action.clearError();
  }, [base]);
  const save = async () => {
    if (parseError || !dirty) return !dirty;
    const expectedDraft = draftRevision.current;
    return action.run(async () => {
      if (!base.initialized)
        throw new Error('资料库尚未初始化。请先到“状态与诊断”明确完成首次设置。');
      if (!known) throw new Error('已保存的配置与偏好尚未确认；请重新读取配置，不能保存推测值。');
      const check = await bridge.call('config_validate', { patch });
      setValidation(check);
      if (!check.valid) throw new Error('配置校验未通过，请修正标注的字段。');
      const result = await bridge.call('config_save', {
        admission: await admission.obtain('config_cas_v1'),
        expectedRevision: base.revision,
        patch,
      });
      setBase(result);
      if (draftRevision.current === expectedDraft) setDraft(configDraft(result.values));
      return '配置已原子保存；每个字段的生效时机见校验结果。';
    });
  };
  useEffect(() => {
    register({ dirty, save, discard: reset });
    return () => register(null);
  });
  const groups = [...new Set(Object.values(fields).map((f) => f.group))];
  return (
    <>
      <Paper className="panel settings-summary" withBorder p="md" component="section">
        {!base.initialized && (
          <Alert className="notice" variant="light" role="status">
            资料库尚未初始化；下方仅展示未保存的默认值，不能编辑或保存。请先打开“状态与诊断”完成首次设置，不会自动创建配置。
          </Alert>
        )}
        <Text component="p">
          自动录音：{base.autoRecord === null ? '未知' : base.autoRecord ? '开' : '关'} · 录音同意：
          {base.recordingConsent === null ? '未知' : base.recordingConsent ? '已确认' : '未确认'}
          （通过顶部录音入口显式更改）
        </Text>
        {base.initialized && !known && (
          <Alert className="notice" variant="light" role="status">
            配置保存状态未能确认；下方值不能视为已保存配置，暂不可编辑或保存。请重新读取配置。
          </Alert>
        )}
        <Text component="p">
          {known ? (
            <>
              模型版本：{base.modelVersion} ·{' '}
              {base.structurallyComplete ? '结构完整' : '配置不完整'}
            </>
          ) : (
            '模型与配置结构状态未知'
          )}
        </Text>
        {base.warnings.map((w, i) => (
          <Alert className="notice" key={i} variant="light" role="status">
            {w}
          </Alert>
        ))}
        <Text className="muted" component="p" c="dimmed">
          Pi 认证由 Pi 自己管理。这里不读取、显示或保存凭据。
        </Text>
      </Paper>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <div className="settings-grid">
          {groups.map((group) => (
            <Fieldset className="panel" key={group} disabled={action.busy || !known} legend={group}>
              {(Object.entries(fields) as Array<[keyof EditableConfig, Field]>)
                .filter(([, f]) => f.group === group)
                .map(([key, f]) => {
                  const error = validation?.fieldErrors
                    .filter((entry) => entry.field === key || entry.field.endsWith(`.${key}`))
                    .map((entry) => entry.message)
                    .join(' ');
                  const common = { label: f.label, description: f.help, error: error || undefined };
                  return (
                    <div key={key} className="settings-field">
                      {f.type === 'boolean' ? (
                        <Switch
                          {...common}
                          checked={draft[key] === true}
                          onChange={(e) => setDraft((d) => ({ ...d, [key]: e.target.checked }))}
                        />
                      ) : f.type === 'lines' || f.type === 'events' ? (
                        <Textarea
                          {...common}
                          value={String(draft[key])}
                          onChange={(e) => setDraft((d) => ({ ...d, [key]: e.target.value }))}
                        />
                      ) : (
                        <TextInput
                          {...common}
                          inputMode={f.type === 'number' ? 'numeric' : 'text'}
                          value={String(draft[key])}
                          onChange={(e) => setDraft((d) => ({ ...d, [key]: e.target.value }))}
                        />
                      )}
                    </div>
                  );
                })}
            </Fieldset>
          ))}
        </div>
        <Paper className="panel settings-save" withBorder p="md" component="section">
          <span role="status">
            {dirty ? '有未保存的修改' : '当前没有未保存的修改'}
            {validating && ' · 校验中…'}
          </span>
          {!!validationError && <ErrorNotice error={validationError} />}{' '}
          {!!action.error && <ErrorNotice error={action.error} />}
          {validation && !validation.valid && (
            <div role="alert">
              {validation.fieldErrors.map((e, i) => (
                <Text key={i} component="p">
                  {e.field}：{e.message}
                </Text>
              ))}
            </div>
          )}
          {validation && validation.effects.length > 0 && (
            <Disclosure title="配置生效时机">
              <ul>
                {validation.effects.map((e) => (
                  <li key={e.field}>
                    {fields[e.field as keyof EditableConfig]?.label ?? e.field}：
                    {effectName[e.takesEffect]}
                  </li>
                ))}
              </ul>
            </Disclosure>
          )}
          <div className="button-row">
            <Button
              type="button"
              disabled={!dirty || action.busy}
              onClick={() => setDiscard(true)}
              variant="default"
            >
              放弃修改
            </Button>
            <Button
              type="button"
              disabled={action.busy}
              onClick={() => setDiscard(true)}
              variant="default"
            >
              重新读取配置
            </Button>
            <Button
              className="primary"
              disabled={
                !dirty || action.busy || validating || !!parseError || validation?.valid !== true
              }
              type="submit"
              variant="filled"
            >
              保存配置
            </Button>
          </div>
        </Paper>
      </form>
      <Disclosure className="panel section-spaced" title="本机数据位置">
        <dl>
          {Object.entries(base.roots).map(([key, value]) => (
            <div key={key}>
              <dt>{key}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      </Disclosure>
      {discard && (
        <Modal title="放弃草稿并重新读取？" close={() => setDiscard(false)} busy={action.busy}>
          <Text component="p">
            其他进程可能已经修改配置。重新读取将丢弃这里尚未保存的草稿，不会覆盖磁盘内容。版本冲突时草稿会一直保留到你确认放弃。
          </Text>
          <div className="modal-actions">
            <Button
              disabled={action.busy}
              onClick={() => setDiscard(false)}
              type="button"
              variant="default"
            >
              保留草稿
            </Button>
            <Button
              disabled={action.busy}
              onClick={() =>
                void action.run(async () => {
                  const current = await bridge.call('config_get', {});
                  setBase(current);
                  setDraft(configDraft(current.values));
                  setDiscard(false);
                })
              }
              type="button"
              variant="default"
            >
              放弃草稿并读取
            </Button>
          </div>
        </Modal>
      )}
    </>
  );
}
