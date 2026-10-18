import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { focusManager, QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BridgeContext } from '../src/app/context';
import { Detail } from '../src/features/detail';
import type { ArtifactVersion, MeetingStatus } from '../src/contracts.generated';
import { fixtureBridge, meetingFixture, sample } from './fixtures';

const taskId = '00000000-0000-4000-8000-000000000042';
const oldId = '00000000-0000-4000-8000-000000000043';
let visibility: DocumentVisibilityState;
let originalVisibility: PropertyDescriptor | undefined;
async function tick(ms = 10) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}
async function visible(value: DocumentVisibilityState) {
  await act(async () => {
    visibility = value;
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
    await vi.advanceTimersByTimeAsync(10);
  });
  expect(focusManager.isFocused()).toBe(value === 'visible');
}
function fixture(status: 'recording' | 'unsealed' = 'recording', history = false) {
  let phase = 0;
  let meetingStatus: MeetingStatus = status;
  const base = {
    ...meetingFixture(),
    currentTranscriptTaskId: null,
    currentSummaryTaskId: null,
  };
  const artifact = { meetingId: base.meetingId, taskId, stage: 'transcribe' as const };
  const version = (historical = false): ArtifactVersion => ({
    artifact: historical ? { ...artifact, taskId: oldId } : artifact,
    attempt: historical ? 1 : 2,
    status: historical ? 'historical' : phase === 2 ? 'committed' : 'partial',
    taskStatus: historical || phase === 2 ? 'completed' : 'transcribing',
    sha256: historical ? 'f'.repeat(64) : String(phase + 1).repeat(64),
    byteLength: 100,
    createdAtMs: 1,
    sourceTranscriptTaskId: null,
    sourceTranscriptHash: null,
    modelVersion: 'synthetic',
    configHash: 'a'.repeat(64),
    current: !historical && phase === 2,
  });
  const fake = fixtureBridge({
    meeting_get: () => ({
      ...sample('meeting_get'),
      meeting: {
        ...base,
        status: meetingStatus,
        currentTranscriptTaskId: phase === 2 ? taskId : null,
      },
      hasPartialTranscript: phase === 1,
    }),
    task_list: () => ({ ...sample('task_list'), items: [], nextCursor: null }),
    media_manifest: () => ({ ...sample('media_manifest'), segments: [], nextCursor: null }),
    artifact_versions: () => ({
      ...sample('artifact_versions'),
      items: [...(phase ? [version()] : []), ...(history ? [version(true)] : [])],
      revision: String(phase + 1).repeat(64),
      nextCursor: null,
    }),
    artifact_read: (p) => {
      const historical = p.artifact.taskId === oldId;
      const text = historical ? '保留的历史正文' : phase === 2 ? '原地终稿正文' : '隐藏中首片段';
      return {
        ...sample('artifact_read'),
        artifact: p.artifact,
        revision: historical ? 'f'.repeat(64) : String(phase + 1).repeat(64),
        status: historical ? 'historical' : phase === 2 ? 'committed' : 'partial',
        text,
        lines: [{ startMs: 0, endMs: 1000, speaker: 'mix', text }],
        complete: true,
        nextCursor: null,
      };
    },
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 1000, gcTime: 60000 } },
  });
  const view = render(
    <BridgeContext.Provider value={fake.bridge}>
      <QueryClientProvider client={client}>
        <Detail id={base.meetingId} back={() => undefined} openMeeting={() => undefined} />
      </QueryClientProvider>
    </BridgeContext.Provider>,
  );
  return {
    ...fake,
    ...view,
    phase: (value: number) => {
      phase = value;
    },
    status: (value: MeetingStatus) => {
      meetingStatus = value;
    },
    requests: () => {
      const count = (command: string) => fake.calls.filter((c) => c.command === command).length;
      return {
        meeting_get: count('meeting_get'),
        artifact_versions: count('artifact_versions'),
        artifact_read: count('artifact_read'),
      };
    },
  };
}
beforeEach(() => {
  vi.useFakeTimers();
  visibility = 'visible';
  originalVisibility = Object.getOwnPropertyDescriptor(document, 'visibilityState');
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => visibility,
  });
  focusManager.setFocused(undefined);
});
afterEach(() => {
  cleanup();
  if (originalVisibility) Object.defineProperty(document, 'visibilityState', originalVisibility);
  else Reflect.deleteProperty(document, 'visibilityState');
  focusManager.setFocused(undefined);
  vi.useRealTimers();
});

describe('Detail and Artifacts authoritative hidden terminal polling', () => {
  it('discovers a hidden first segment, stops after sealing, then preserves the mounted search when visible', async () => {
    const f = fixture();
    await tick();
    fireEvent.click(screen.getByRole('tab', { name: '逐字稿' }));
    await tick();
    const panel = screen.getByRole('tabpanel');
    const back = screen.getByRole('button', { name: '← 返回会议库' });
    await visible('hidden');
    await tick(6000);
    expect(screen.getByText(/正在准备部分逐字稿/)).toBeVisible();
    f.phase(1);
    await tick(4500);
    expect(screen.getByText('隐藏中首片段')).toBeVisible();
    const search = screen.getByLabelText('当前已载入逐字稿内搜索');
    fireEvent.change(search, { target: { value: '正文' } });
    f.phase(2);
    f.status('completed');
    await tick(12000);
    expect(screen.getByText('原地终稿正文')).toBeVisible();
    expect(screen.queryByText(/当前内容不是终稿/)).not.toBeInTheDocument();
    const terminal = f.requests();
    await tick(30000);
    expect(f.requests()).toEqual(terminal);
    await visible('visible');
    await tick();
    expect(screen.getByRole('tabpanel')).toBe(panel);
    expect(screen.getByRole('button', { name: '← 返回会议库' })).toBe(back);
    expect(screen.getByLabelText('当前已载入逐字稿内搜索')).toBe(search);
    expect(search).toHaveValue('正文');
    expect(screen.getByText('原地终稿正文')).toBeVisible();
    const foreground = f.requests().meeting_get;
    await tick(11000);
    expect(f.requests().meeting_get).toBeGreaterThan(foreground);
    f.unmount();
    const unmounted = f.requests();
    await tick(20000);
    expect(f.requests()).toEqual(unmounted);
  });
  it('resumes ordinary foreground refresh even when shown again before the terminal snapshot becomes stale', async () => {
    const f = fixture();
    await tick();
    fireEvent.click(screen.getByRole('tab', { name: '逐字稿' }));
    await visible('hidden');
    f.phase(2);
    f.status('completed');
    await tick(2050);
    expect(screen.getByText('原地终稿正文')).toBeVisible();
    const terminal = f.requests().meeting_get;
    await visible('visible');
    await tick(11000);
    expect(f.requests().meeting_get).toBeGreaterThan(terminal);
  });
  it('keeps discovering while unsealed with only terminal history, then stops without losing that selected history', async () => {
    const f = fixture('unsealed', true);
    await tick();
    fireEvent.click(screen.getByRole('tab', { name: '处理版本' }));
    await tick();
    fireEvent.click(
      within(document.querySelector('.version-row') as HTMLElement).getByRole('button', {
        name: '读取此版本',
      }),
    );
    await tick();
    const panel = screen.getByRole('tabpanel');
    const search = screen.getByLabelText('当前已载入逐字稿内搜索');
    fireEvent.change(search, { target: { value: '历史' } });
    await visible('hidden');
    const before = f.requests();
    await tick(6500);
    expect(f.requests().artifact_versions).toBeGreaterThan(before.artifact_versions);
    f.phase(1);
    await tick(4500);
    expect(screen.getByText('保留的历史正文')).toBeVisible();
    f.phase(2);
    f.status('completed');
    await tick(12000);
    const terminal = f.requests();
    await tick(30000);
    expect(f.requests()).toEqual(terminal);
    await visible('visible');
    await tick();
    expect(screen.getByRole('tabpanel')).toBe(panel);
    expect(screen.getByText('保留的历史正文')).toBeVisible();
    expect(screen.queryByText('原地终稿正文')).not.toBeInTheDocument();
    expect(screen.getByLabelText('当前已载入逐字稿内搜索')).toBe(search);
    expect(search).toHaveValue('历史');
  });
  it('stops hidden unsealed discovery when the authoritative meeting fails without any versions', async () => {
    const f = fixture('unsealed');
    await tick();
    fireEvent.click(screen.getByRole('tab', { name: '逐字稿' }));
    await visible('hidden');
    await tick(6500);
    expect(screen.getByText(/正在准备部分逐字稿/)).toBeVisible();
    f.status('failed');
    await tick(12000);
    expect(screen.queryByText(/正在准备部分逐字稿/)).not.toBeInTheDocument();
    const terminal = f.requests();
    await tick(30000);
    expect(f.requests()).toEqual(terminal);
  });
});
