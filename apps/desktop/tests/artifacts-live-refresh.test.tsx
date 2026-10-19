import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BridgeContext } from '../src/app/context';
import { Artifacts } from '../src/features/artifacts';
import { conflict, fixtureBridge, meetingFixture, sample } from './fixtures';
import type { ArtifactVersion } from '../src/contracts.generated';

const meeting = { ...meetingFixture(), status: 'queued' as const, currentTranscriptTaskId: null };
const taskId = '00000000-0000-4000-8000-000000000042';
const oldId = '00000000-0000-4000-8000-000000000043';
const artifact = { meetingId: meeting.meetingId, taskId, stage: 'transcribe' as const };
const revision = (phase: number) => String(phase + 1).repeat(64);
async function tick(ms = 10) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}
function fixture(initial = 0, sameFinalBytes = false) {
  let phase = initial;
  let rejectCursor = false;
  let holdCursor = false;
  let releaseCursor: (() => void) | undefined;
  const bodyPhase = () => (sameFinalBytes && phase === 3 ? 2 : phase);
  const version = (historical = false): ArtifactVersion => ({
    artifact: historical ? { ...artifact, taskId: oldId } : artifact,
    attempt: historical ? 1 : 2,
    status: historical
      ? 'historical'
      : phase === 0
        ? 'missing'
        : phase === 3
          ? 'committed'
          : 'partial',
    taskStatus: historical || phase === 3 ? 'completed' : 'transcribing',
    sha256: historical ? 'f'.repeat(64) : phase ? revision(bodyPhase()) : null,
    byteLength: phase ? 100 : null,
    createdAtMs: 1,
    sourceTranscriptTaskId: null,
    sourceTranscriptHash: null,
    modelVersion: 'synthetic',
    configHash: 'a'.repeat(64),
    current: !historical && phase === 3,
  });
  const fake = fixtureBridge({
    artifact_versions: () => ({
      ...sample('artifact_versions'),
      items: [version(), version(true)],
      revision: revision(phase),
      nextCursor: null,
    }),
    artifact_read: (p) => {
      if (p.cursor && (rejectCursor || p.cursor !== `snapshot_${phase}_cursor_000001`))
        throw conflict();
      const historical = p.artifact.taskId === oldId;
      const text = historical
        ? '保留的历史正文'
        : p.cursor
          ? `快照${bodyPhase()}尾页`
          : `快照${bodyPhase()}首片段`;
      const response = {
        ...sample('artifact_read'),
        artifact: p.artifact,
        revision: historical ? 'f'.repeat(64) : revision(phase),
        status: historical
          ? ('historical' as const)
          : phase === 3
            ? ('committed' as const)
            : ('partial' as const),
        text,
        lines: [{ startMs: 0, endMs: 1000, speaker: 'mix' as const, text }],
        complete: historical || !!p.cursor || phase === 3,
        nextCursor:
          !historical && !p.cursor && phase !== 3 ? `snapshot_${phase}_cursor_000001` : null,
      };
      return holdCursor && p.cursor
        ? new Promise<typeof response>((resolve) => {
            releaseCursor = () => resolve(response);
          })
        : response;
    },
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 1000, gcTime: 0 } },
  });
  const view = render(
    <BridgeContext.Provider value={fake.bridge}>
      <QueryClientProvider client={client}>
        <Artifacts
          meeting={meeting}
          partial={false}
          locate={() => undefined}
          onTranscript={() => undefined}
        />
      </QueryClientProvider>
    </BridgeContext.Provider>,
  );
  return {
    ...fake,
    ...view,
    phase: (value: number) => {
      phase = value;
    },
    reject: () => {
      rejectCursor = true;
    },
    hold: () => {
      holdCursor = true;
    },
    release: () => releaseCursor?.(),
  };
}
async function transcript() {
  await tick();
  fireEvent.click(screen.getByRole('tab', { name: '逐字稿' }));
  await tick();
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('Artifacts mounted live partial refresh and snapshot isolation', () => {
  it('discovers the first committed segment without remount, tab switch or focus', async () => {
    const f = fixture();
    await transcript();
    expect(screen.getByText(/正在准备部分逐字稿/)).toBeVisible();
    const panel = screen.getByRole('tabpanel');
    f.phase(1);
    await tick(2200);
    expect(screen.getByText('快照1首片段')).toBeVisible();
    expect(screen.getByText(/当前内容不是终稿/)).toBeVisible();
    expect(screen.getByRole('tabpanel')).toBe(panel);
  });
  it('replaces loaded pages on an advancing snapshot, then removes the partial notice in place', async () => {
    const f = fixture(1);
    await transcript();
    fireEvent.click(screen.getByRole('button', { name: '载入更多内容' }));
    await tick();
    expect(screen.getByText('快照1尾页')).toBeVisible();
    const panel = screen.getByRole('tabpanel');
    const search = screen.getByLabelText('当前已载入逐字稿内搜索');
    fireEvent.change(search, { target: { value: '快照' } });
    f.phase(2);
    await tick(2200);
    expect(screen.getByText('快照2首片段')).toBeVisible();
    expect(screen.queryByText('快照1尾页')).not.toBeInTheDocument();
    expect(screen.queryByText('快照1首片段')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '载入更多内容' }));
    await tick();
    expect(screen.getByText('快照2尾页')).toBeVisible();
    f.phase(3);
    await tick(2200);
    expect(screen.getByText('快照3首片段')).toBeVisible();
    expect(screen.queryByText(/当前内容不是终稿/)).not.toBeInTheDocument();
    expect(screen.queryByText('快照2尾页')).not.toBeInTheDocument();
    expect(screen.getByRole('tabpanel')).toBe(panel);
    expect(screen.getByLabelText('当前已载入逐字稿内搜索')).toBe(search);
    expect(search).toHaveValue('快照');
  });
  it('refreshes partial to final even when the final content hash is unchanged', async () => {
    const f = fixture(2, true);
    await transcript();
    expect(screen.getByText('快照2首片段')).toBeVisible();
    expect(screen.getByText(/当前内容不是终稿/)).toBeVisible();
    f.phase(3);
    await tick(2200);
    expect(screen.getByText('快照2首片段')).toBeVisible();
    expect(screen.queryByText(/当前内容不是终稿/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '载入更多内容' })).not.toBeInTheDocument();
  });
  it('drops a late previous-snapshot page after the current snapshot has replaced it', async () => {
    const f = fixture(1);
    await transcript();
    f.hold();
    fireEvent.click(screen.getByRole('button', { name: '载入更多内容' }));
    await tick();
    f.phase(2);
    await tick(2200);
    expect(screen.getByText('快照2首片段')).toBeVisible();
    f.release();
    await tick();
    expect(screen.getByText('快照2首片段')).toBeVisible();
    expect(screen.queryByText('快照1首片段')).not.toBeInTheDocument();
    expect(screen.queryByText('快照1尾页')).not.toBeInTheDocument();
    expect(screen.getAllByText('快照2首片段')).toHaveLength(1);
  });
  it('resets a conflicting cursor to the new head rather than appending a duplicate old head', async () => {
    const f = fixture(1);
    await transcript();
    f.phase(2);
    f.reject();
    fireEvent.click(screen.getByRole('button', { name: '载入更多内容' }));
    await tick(50);
    expect(screen.getByText('快照2首片段')).toBeVisible();
    expect(screen.queryByText('快照1首片段')).not.toBeInTheDocument();
    expect(screen.getAllByText('快照2首片段')).toHaveLength(1);
    expect(
      f.calls.some(
        (c) =>
          c.command === 'artifact_read' &&
          (c.payload as { cursor: string }).cursor === 'snapshot_1_cursor_000001',
      ),
    ).toBe(true);
    expect(screen.getByRole('button', { name: '载入更多内容' })).toBeEnabled();
  });
  it('stops live artifact requests once the server reports terminal versions, and after unmount', async () => {
    const f = fixture(1);
    await transcript();
    f.phase(3);
    await tick(2200);
    expect(screen.getByText('快照3首片段')).toBeVisible();
    const reads = () =>
      f.calls.filter((c) => ['artifact_versions', 'artifact_read'].includes(c.command)).length;
    const terminal = reads();
    await tick(8000);
    expect(reads()).toBe(terminal);
    f.unmount();
    await tick(8000);
    expect(reads()).toBe(terminal);
  });
  it('stops active polling on unmount and preserves an explicitly selected historical version', async () => {
    const f = fixture(1);
    await tick();
    fireEvent.click(screen.getByRole('tab', { name: '处理版本' }));
    await tick();
    const historical = document.querySelectorAll('.version-row')[1] as HTMLElement;
    fireEvent.click(within(historical).getByRole('button', { name: '读取此版本' }));
    await tick();
    expect(screen.getByText('保留的历史正文')).toBeVisible();
    f.phase(2);
    await tick(2200);
    expect(screen.getByText('保留的历史正文')).toBeVisible();
    expect(screen.queryByText('快照2首片段')).not.toBeInTheDocument();
    expect(
      f.calls.filter(
        (c) =>
          c.command === 'artifact_read' &&
          (c.payload as { artifact: { taskId: string } }).artifact.taskId === oldId,
      ),
    ).toHaveLength(1);
    f.unmount();
    const count = f.calls.length;
    await tick(8000);
    expect(f.calls).toHaveLength(count);
  });
});
