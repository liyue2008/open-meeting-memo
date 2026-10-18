// Synthetic transport only. This fixture never imports the production Tauri transport.
import { createRoot } from 'react-dom/client';
import { App } from '../../src/App';
import { createBridge } from '../../src/bridge/client';
import * as validators from '../../src/bridge/validators.generated.js';
import '../../src/styles.css';
const samples = window.__samples;
const token = 'LONGTOKEN'.repeat(20);
window.__mode = 'normal';
window.__calls = [];
window.__playCalls = 0;
window.__copiedGuidance = '';
Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: {
    writeText: async (text) => {
      window.__copiedGuidance = text;
    },
  },
});
window.__pending = [];
window.__invalidResponses = [];
HTMLMediaElement.prototype.play = function () {
  window.__playCalls++;
  throw new Error('Hardware playback forbidden in layout fixture');
};
const mid = samples.meeting_get.meeting.meetingId;
const tid = samples.task_get.task.taskId;
const rev = 'a'.repeat(64);
const failure = {
  code: 'unavailable',
  reason: null,
  retryable: false,
  message: '合成读取错误 ' + token,
  fieldErrors: [],
};
window.__bridgeFailures = [];
const checkedBridge = createBridge(async (command, { request }) => {
  window.__calls.push(command);
  const mode = window.__mode;
  const scenario = window.__scenario || {};
  if (scenario.pendingCommands?.includes(command))
    return new Promise((resolve) =>
      window.__pending.push(() =>
        resolve({ schemaVersion: 1, requestId: request.requestId, error: failure }),
      ),
    );
  let data = structuredClone(samples[command]);
  const fail =
    mode === 'error' &&
    [
      'meeting_list',
      'task_list',
      'meeting_get',
      'config_get',
      'diagnostics_get',
      'notification_history',
      'service_plan',
    ].includes(command);
  if (
    fail ||
    scenario.failedCommands?.includes(command) ||
    (window.__failAction &&
      [
        'meeting_update',
        'meeting_trash',
        'meeting_restore',
        'processing_enqueue',
        'processing_retry',
        'processing_cancel',
        'processing_requeue',
        'config_save',
        'service_apply',
        'profile_initialize',
        'import_select',
        'import_commit',
        'record_start',
        'record_auto_set',
        'diagnostics_export',
      ].includes(command))
  ) {
    return { schemaVersion: 1, requestId: request.requestId, error: failure };
  }
  const meeting = {
    ...samples.meeting_get.meeting,
    metadata: {
      title: mode === 'long' ? '合成长标题 ' + token : '合成会议',
      note: mode === 'long' ? 'synthetic/path/'.repeat(40) : '合成备注',
      tags: ['合成'],
    },
    startedAtMs: Date.now(),
    status: scenario.meetingStatus || 'completed',
    durationMs: 60000,
    currentSummaryTaskId: tid,
    currentTranscriptTaskId: tid,
    capabilities: { edit: true, trash: true, restore: true, reprocess: true, reason: null },
  };
  const task = {
    ...samples.task_get.task,
    status: scenario.taskStatus || 'failed',
    canRetry: true,
    canCancel: true,
    canRequeue: true,
    error: { ...failure, message: mode === 'long' ? token : '合成任务错误' },
  };
  if (command === 'meeting_list')
    data = {
      ...data,
      items:
        mode === 'empty'
          ? []
          : [{ ...meeting, trashed: request.payload.filter.visibility === 'trash' }],
      total: mode === 'empty' ? 0 : 1,
      nextCursor: null,
    };
  if (command === 'meeting_get') data.meeting = meeting;
  if (command === 'task_list')
    data = { ...data, items: mode === 'empty' ? [] : [task], nextCursor: null };
  if (command === 'task_get')
    data = { ...data, task, diagnostics: [mode === 'long' ? token : '合成脱敏信息'] };
  if (command === 'artifact_versions')
    data.items =
      mode === 'empty'
        ? []
        : ['summary', 'transcribe'].flatMap((stage) =>
            [true, false].map((current) => ({
              artifact: {
                meetingId: mid,
                taskId: current ? tid : '00000000-0000-4000-8000-000000000009',
                stage,
              },
              attempt: current ? 2 : 1,
              status: current ? 'committed' : 'historical',
              taskStatus: 'completed',
              sha256: rev,
              byteLength: 200,
              createdAtMs: 1,
              sourceTranscriptTaskId: stage === 'summary' ? tid : null,
              sourceTranscriptHash: stage === 'summary' ? rev : null,
              modelVersion: mode === 'long' ? token : 'synthetic',
              configHash: rev,
              current,
            })),
          );
  if (command === 'artifact_read')
    data = {
      ...data,
      artifact: request.payload.artifact,
      status: mode === 'empty' ? 'missing' : 'committed',
      format: request.payload.artifact.stage === 'summary' ? 'markdown' : 'transcript',
      text:
        mode === 'empty'
          ? ''
          : '# 合成纪要\n[00:01] 定位\n\n' +
            (mode === 'long'
              ? token +
                '\n\n|字段|内容|\n|---|---|\n|合成|' +
                token +
                '|\n\n```\n' +
                token +
                '\n```'
              : '仅供布局检查') +
            '\n## 行动项\n- [ ] 合成待办',
      lines:
        mode === 'empty'
          ? []
          : [
              {
                startMs: 1000,
                endMs: 2000,
                speaker: 'mix',
                text: mode === 'long' ? token : '合成逐字稿',
              },
            ],
      complete: true,
      nextCursor: null,
    };
  if (command === 'config_get') {
    data.initialized = mode !== 'empty';
    data.revision = rev;
    if (mode === 'long') {
      for (const key of Object.keys(data.values))
        if (/Path|Executable/.test(key))
          data.values[key] = '/synthetic/' + 'long_path_segment/'.repeat(30);
      for (const key of Object.keys(data.roots))
        data.roots[key] = '/synthetic/' + 'long_path_segment/'.repeat(30);
      data.warnings = [token];
    }
  }
  if (command === 'config_validate')
    data = {
      ...data,
      valid: true,
      fieldErrors: [],
      effects: [{ field: 'piProvider', takesEffect: 'new_task' }],
    };
  if (command === 'service_plan') {
    data.action = request.payload.action;
    if (scenario.applyAllowed) {
      data.canApply = true;
      data.planToken = 'fixture_lifecycle_plan_000000000001';
      data.reasons = ['ready'];
    }
    data.expiresAtMs = Date.now() + 60000;
    if (mode === 'empty')
      data = {
        ...data,
        state: 'uninitialized',
        canInitialize: true,
        canApply: false,
        planToken: null,
        expiresAtMs: null,
        reasons: ['profile_not_initialized'],
      };
    if (mode === 'unknown')
      data = {
        ...data,
        state: 'unknown',
        canInitialize: false,
        canApply: false,
        planToken: null,
        expiresAtMs: null,
        reasons: ['unowned_service'],
      };
    if (mode === 'busy')
      data = {
        ...data,
        state: 'busy',
        canApply: false,
        planToken: null,
        expiresAtMs: null,
        reasons: ['service_busy'],
      };
  }
  if (command === 'diagnostics_get')
    data.items =
      mode === 'empty'
        ? []
        : [
            {
              key: 'synthetic-dependency',
              state: 'unknown',
              code: null,
              message: mode === 'long' ? token : '未检查真实权限或模型',
              action: mode === 'long' ? token : '仅合成状态',
            },
          ];
  if (command === 'artifact_export') data = { outcome: 'saved', fileName: 'synthetic.md' };
  if (command === 'diagnostics_export')
    data = { outcome: 'saved', fileName: 'synthetic-diagnostics.json' };
  if (command === 'import_select')
    data.selection = {
      selectionHandle: 'fixture_selection_000001',
      displayName: 'synthetic-' + token + '.wav',
      byteLength: 32044,
      format: 'wav',
      durationMs: 1000,
      channels: 1,
      sampleRate: 16000,
      expiresAtMs: Date.now() + 60000,
    };
  if (scenario.recordState && command === 'record_status') data.state = scenario.recordState;
  if (scenario.readOnly && data?.compatibility)
    data.compatibility = {
      ...data.compatibility,
      state: 'read_only',
      writesAllowed: false,
      admissionMode: 'none',
      reason: 'incompatible_service',
      writeAdmission: null,
      capabilities: [],
    };
  if (scenario.artifactStatus && command === 'artifact_read') {
    data.status = scenario.artifactStatus;
    data.complete = scenario.artifactStatus !== 'partial';
  }
  if (scenario.invalidConfig && command === 'config_validate') {
    data.valid = false;
    data.fieldErrors = Object.keys(samples.config_get.values)
      .slice(0, scenario.invalidConfigCount || 1)
      .map((field) => ({ field, message: token }));
  }
  if (scenario.recordPrefs && command === 'record_status')
    Object.assign(data, scenario.recordPrefs);
  if (scenario.configPrefs && command === 'config_get') Object.assign(data, scenario.configPrefs);
  if (scenario.recoveryPlan && command === 'service_plan') {
    Object.assign(data, scenario.recoveryPlan, { action: request.payload.action });
    if (scenario.expired) data.expiresAtMs = Date.now() - 1;
    if (scenario.adoption) {
      data.canApply = window.__adopted || request.payload.action === 'switch';
      if (window.__adopted) {
        data.reasons = ['ready'];
        data.effects = ['Start verified bundled service using unchanged saved preferences'];
        data.guidance = null;
      }
    }
  }
  if (scenario.adoption && command === 'service_apply') {
    data.action = window.__adopted ? 'start' : 'switch';
    data.state = window.__adopted ? 'running' : 'stopped';
    window.__adopted = true;
  }
  const response = { schemaVersion: 1, requestId: request.requestId, data };
  if (!validators[command + '_response'](response)) window.__invalidResponses.push(command);
  return response;
});
const bridge = {
  async call(command, payload, signal) {
    try {
      return await checkedBridge.call(command, payload, signal);
    } catch (e) {
      window.__bridgeFailures.push({ command, message: e.message, reason: e.detail?.reason });
      throw e;
    }
  },
};
const root = createRoot(document.getElementById('root'));
let epoch = 0;
window.__remount = () => {
  window.__pending.splice(0).forEach((resolve) => resolve());
  window.__failAction = false;
  root.render(<App key={++epoch} bridge={bridge} />);
};
window.__remount();
