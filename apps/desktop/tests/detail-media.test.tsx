import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { App } from '../src/App';
import { fixtureBridge, meetingFixture, sample } from './fixtures';
import type { ArtifactVersion, MediaSegment } from '../src/contracts.generated';
const mid = meetingFixture().meetingId;
const task1 = '00000000-0000-4000-8000-000000000002';
const task2 = '00000000-0000-4000-8000-000000000003';
const old = '00000000-0000-4000-8000-000000000004';
const historicalTranscript = '00000000-0000-4000-8000-000000000005';
const revision = 'a'.repeat(64);
const segment = (sequence: number, offsetMs: number): MediaSegment => ({
  segmentId: `00000000-0000-4000-8000-00000000001${sequence}`,
  sequence,
  offsetMs,
  durationMs: 20000,
  mime: 'audio/wav',
  byteLength: 640044,
  sha256: revision,
  available: true,
  reason: null,
});
const version = (
  stage: 'summary' | 'transcribe',
  taskId: string,
  current = true,
): ArtifactVersion => ({
  artifact: { meetingId: mid, taskId, stage },
  attempt: current ? 2 : 1,
  status: current ? 'committed' : 'historical',
  taskStatus: 'completed',
  sha256: revision,
  byteLength: 100,
  createdAtMs: 1,
  sourceTranscriptTaskId: stage === 'summary' ? task1 : null,
  sourceTranscriptHash: stage === 'summary' ? revision : null,
  modelVersion: 'synthetic',
  configHash: revision,
  current,
});
function fixture() {
  const meeting = {
    ...meetingFixture(),
    durationMs: 60000,
    currentSummaryTaskId: task2,
    currentTranscriptTaskId: task1,
  };
  return fixtureBridge({
    meeting_list: () => ({ ...sample('meeting_list'), items: [meeting], total: 1 }),
    meeting_get: () => ({ ...sample('meeting_get'), meeting }),
    media_manifest: () => ({
      meetingId: mid,
      revision,
      segments: [segment(0, 0), segment(1, 30000)],
      nextCursor: null,
      durationMs: 60000,
      complete: true,
    }),
    media_open: (p) => ({
      ...sample('media_open'),
      grantId: `fixture_media_grant_00000${p.segmentId.endsWith('0') ? 1 : 2}`,
      url: `omm-media://localhost/fixture_media_grant_00000${p.segmentId.endsWith('0') ? 1 : 2}`,
      revision,
    }),
    artifact_versions: () => ({
      ...sample('artifact_versions'),
      items: [
        version('summary', task2),
        version('transcribe', task1),
        version('summary', old, false),
        version('transcribe', historicalTranscript, false),
      ],
    }),
    artifact_read: (p) => ({
      ...sample('artifact_read'),
      artifact: p.artifact,
      status: p.artifact.taskId === old ? 'historical' : 'committed',
      format: p.artifact.stage === 'summary' ? 'markdown' : 'transcript',
      text:
        p.artifact.stage === 'summary'
          ? `${p.artifact.taskId === old ? '# 历史合成纪要' : '# 合成纪要'}\n[00:35] 定位\n## 行动项\n- [ ] 甲准备合成材料\n<script>throw new Error('forbidden')</script>\n![leak](https://untrusted.invalid/pixel)\n[unsafe](javascript:alert(1))`
          : '',
      lines:
        p.artifact.stage === 'transcribe'
          ? [{ startMs: 35000, endMs: 40000, speaker: 'mix', text: '合成逐字稿片段' }]
          : [],
      complete: true,
      nextCursor: null,
    }),
    artifact_copy: () => ({ acknowledged: true }),
    artifact_export: () => ({ outcome: 'saved', fileName: 'synthetic.md' }),
    processing_enqueue: (p) => ({
      items: p.meetingIds.map((meetingId) => ({
        meetingId,
        taskId: task2,
        operationId: null,
        error: null,
      })),
    }),
  });
}
async function openDetail(fake: ReturnType<typeof fixture>) {
  const result = render(<App bridge={fake.bridge} />);
  await userEvent.click(await screen.findByRole('button', { name: '合成设计讨论' }));
  await screen.findByRole('heading', { name: '合成设计讨论' });
  return result;
}
describe('Detail source artifacts and paused segmented media through typed bridge', () => {
  it('renders safe read-only minutes, native copy/export, old versions and transcript locators', async () => {
    const fake = fixture();
    const { container } = await openDetail(fake);
    await screen.findByRole('heading', { name: '合成纪要' });
    expect(
      container
        .querySelector('.audio-player')!
        .compareDocumentPosition(container.querySelector('.artifact-panel')!) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.getByRole('heading', { name: '行动项 · 只读' })).toBeVisible();
    expect(
      container.querySelectorAll(
        '.artifact-panel input[type=checkbox], .artifact-panel img, .artifact-panel script, .artifact-panel a',
      ),
    ).toHaveLength(0);
    await userEvent.click(screen.getByRole('button', { name: '复制完整内容' }));
    await waitFor(() => expect(fake.calls.some((c) => c.command === 'artifact_copy')).toBe(true));
    await userEvent.click(screen.getByRole('button', { name: '导出' }));
    await waitFor(() => expect(fake.calls.some((c) => c.command === 'artifact_export')).toBe(true));
    await userEvent.click(screen.getByRole('tab', { name: '处理版本' }));
    const rows = container.querySelectorAll('.version-row');
    expect(rows).toHaveLength(4);
    await userEvent.click(
      within(rows[2] as HTMLElement).getByRole('button', { name: '读取此版本' }),
    );
    await screen.findByRole('heading', { name: '历史合成纪要' });
    expect(screen.getByText(/历史产物：保留供查阅/)).toBeVisible();
    await userEvent.click(screen.getByRole('tab', { name: '逐字稿' }));
    await screen.findByText('合成逐字稿片段');
    await userEvent.type(screen.getByLabelText('当前已载入逐字稿内搜索'), '没有匹配');
    expect(screen.queryByText('合成逐字稿片段')).not.toBeInTheDocument();
    await userEvent.clear(screen.getByLabelText('当前已载入逐字稿内搜索'));
    await userEvent.click(screen.getByRole('button', { name: '00:35' }));
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.command === 'media_open').at(-1)?.payload).toMatchObject({
        segmentId: segment(1, 30000).segmentId,
      }),
    );
    await userEvent.click(screen.getByRole('button', { name: '重新处理' }));
    await userEvent.selectOptions(screen.getByLabelText('处理范围'), 'summary');
    await userEvent.click(screen.getByRole('button', { name: '开始处理 1 场' }));
    await waitFor(() =>
      expect(fake.calls.find((c) => c.command === 'processing_enqueue')?.payload).toMatchObject({
        stage: 'summary',
        sourceTranscriptTaskId: task1,
      }),
    );
  });
  it('supports keyboard tabs and binds summary reprocessing to an explicitly selected historical transcript', async () => {
    const fake = fixture();
    const { container } = await openDetail(fake);
    await screen.findByRole('heading', { name: '合成纪要' });
    const summary = screen.getByRole('tab', { name: '会议纪要' });
    summary.focus();
    fireEvent.keyDown(summary, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: '逐字稿' })).toHaveFocus();
    expect(screen.getByRole('tab', { name: '逐字稿' })).toHaveAttribute('aria-selected', 'true');
    fireEvent.keyDown(document.activeElement!, { key: 'End' });
    expect(screen.getByRole('tab', { name: '处理版本' })).toHaveFocus();
    const rows = container.querySelectorAll('.version-row');
    await userEvent.click(
      within(rows[3] as HTMLElement).getByRole('button', { name: '读取此版本' }),
    );
    await screen.findByText('合成逐字稿片段');
    await userEvent.click(screen.getByRole('button', { name: '重新处理' }));
    await userEvent.selectOptions(screen.getByLabelText('处理范围'), 'summary');
    await userEvent.click(screen.getByRole('button', { name: '开始处理 1 场' }));
    await waitFor(() =>
      expect(fake.calls.find((c) => c.command === 'processing_enqueue')?.payload).toMatchObject({
        stage: 'summary',
        sourceTranscriptTaskId: historicalTranscript,
      }),
    );
  });
  it('verifies grant source, seeks paused across gaps/segments and releases every grant', async () => {
    const fake = fixture();
    const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause');
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play');
    const { unmount } = await openDetail(fake);
    await waitFor(() =>
      expect(screen.getByLabelText('录音播放控件')).toHaveAttribute(
        'src',
        'omm-media://localhost/fixture_media_grant_000001',
      ),
    );
    const audio = screen.getByLabelText('录音播放控件') as HTMLAudioElement;
    expect(audio).not.toHaveAttribute('autoplay');
    expect(audio.preload).toBe('metadata');
    Object.defineProperty(audio, 'duration', { configurable: true, value: 20 });
    Object.defineProperty(audio, 'readyState', { configurable: true, value: 1 });
    fireEvent.loadedMetadata(audio);
    fireEvent.change(screen.getByLabelText('会议时间定位'), { target: { value: '10000' } });
    expect(audio.currentTime).toBe(10);
    fireEvent.change(screen.getByLabelText('会议时间定位'), { target: { value: '25000' } });
    expect(await screen.findByText(/此时间位于录音间隙/)).toBeVisible();
    expect(audio.currentTime).toBe(10);
    fireEvent.change(screen.getByLabelText('会议时间定位'), { target: { value: '35000' } });
    await waitFor(() =>
      expect(audio).toHaveAttribute('src', 'omm-media://localhost/fixture_media_grant_000002'),
    );
    fireEvent.loadedMetadata(audio);
    expect(audio.currentTime).toBe(5);
    expect(pause).toHaveBeenCalled();
    expect(play).not.toHaveBeenCalled();
    await userEvent.selectOptions(screen.getByLabelText('速度'), '1.5');
    expect(audio.playbackRate).toBe(1.5);
    unmount();
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.command === 'media_close')).toHaveLength(2),
    );
    expect(play).not.toHaveBeenCalled();
  });
});
