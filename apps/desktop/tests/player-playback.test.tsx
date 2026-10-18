import { seekSlider } from './slider-interaction';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { App } from '../src/App';
import type { DesktopCommandMap } from '../src/contracts.generated';
import { fixtureBridge, meetingFixture, sample } from './fixtures';

function mediaFixture() {
  let grant!: (value: DesktopCommandMap['media_open']['response']) => void;
  const meeting = meetingFixture();
  const fake = fixtureBridge({
    meeting_list: () => ({ ...sample('meeting_list'), items: [meeting], total: 1 }),
    meeting_get: () => ({ ...sample('meeting_get'), meeting }),
    media_manifest: () => ({
      ...sample('media_manifest'),
      meetingId: meeting.meetingId,
      durationMs: 10000,
      segments: [
        {
          segmentId: '00000000-0000-4000-8000-000000000010',
          sequence: 0,
          offsetMs: 0,
          durationMs: 10000,
          mime: 'audio/wav',
          byteLength: 320044,
          sha256: 'a'.repeat(64),
          available: true,
          reason: null,
        },
      ],
    }),
    media_open: () =>
      new Promise((resolve) => {
        grant = resolve;
      }),
  });
  return {
    ...fake,
    resolve: () =>
      grant({
        ...sample('media_open'),
        grantId: 'playback_fixture_grant_00001',
        url: 'omm-media://localhost/playback_fixture_grant_00001',
      }),
  };
}
async function open(fake: ReturnType<typeof mediaFixture>) {
  const view = render(<App bridge={fake.bridge} />);
  await userEvent.click(await screen.findByRole('button', { name: '合成设计讨论' }));
  await waitFor(() => expect(fake.calls.some((c) => c.command === 'media_open')).toBe(true));
  return view;
}
function metadata() {
  const audio = screen.getByLabelText('录音播放控件') as HTMLAudioElement;
  Object.defineProperty(audio, 'duration', { configurable: true, value: 10 });
  Object.defineProperty(audio, 'readyState', { configurable: true, value: 1 });
  fireEvent.loadedMetadata(audio);
  return audio;
}

describe('features/player explicit accessible playback entry (mock media only)', () => {
  it('never plays on grant/metadata/seek; only explicit user controls play and pause', async () => {
    const fake = mediaFixture();
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue();
    const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause');
    const view = await open(fake);
    expect(screen.getByRole('button', { name: '播放录音' })).toBeDisabled();
    fake.resolve();
    await waitFor(() => expect(screen.getByLabelText('录音播放控件')).toHaveAttribute('src'));
    expect(screen.getByRole('button', { name: '播放录音' })).toBeDisabled();
    const audio = metadata();
    expect(audio.controls).toBe(false);
    expect(audio).toHaveAttribute('hidden');
    expect(screen.getAllByRole('slider', { name: '会议时间定位' })).toHaveLength(1);
    expect(screen.getByRole('slider', { name: '会议时间定位' })).toHaveAttribute(
      'aria-valuetext',
      '00:00',
    );
    expect(audio).not.toHaveAttribute('autoplay');
    const timeline = screen.getByRole('slider', { name: '会议时间定位' });
    timeline.focus();
    await userEvent.keyboard('{ArrowRight}');
    expect(audio.currentTime).toBe(0.1);
    expect(timeline).toHaveAttribute('aria-valuetext', '00:00');
    await userEvent.keyboard('{End}');
    expect(audio.currentTime).toBe(10);
    await userEvent.keyboard('{Home}');
    expect(audio.currentTime).toBe(0);
    await seekSlider(1200);
    expect(audio.currentTime).toBe(1.2);
    expect(play).not.toHaveBeenCalled();
    const button = screen.getByRole('button', { name: '播放录音' });
    expect(button).toBeEnabled();
    expect(button).toHaveAttribute('aria-controls', audio.id);
    await userEvent.click(button);
    expect(play).toHaveBeenCalledTimes(1);
    Object.defineProperty(audio, 'paused', { configurable: true, value: false });
    fireEvent.play(audio);
    await userEvent.click(screen.getByRole('button', { name: '暂停录音' }));
    expect(pause).toHaveBeenCalled();
    fireEvent.pause(audio);
    expect(screen.getByRole('button', { name: '播放录音' })).toBeVisible();
    fireEvent.play(audio);
    Object.defineProperty(audio, 'ended', { configurable: true, value: true });
    fireEvent.ended(audio);
    await userEvent.click(screen.getByRole('button', { name: '播放录音' }));
    expect(play).toHaveBeenCalledTimes(2);
    view.unmount();
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.command === 'media_close')).toHaveLength(1),
    );
  });
  it('ignores a late mocked play rejection after the grant is revoked for reopening', async () => {
    const fake = mediaFixture();
    let reject!: (error: Error) => void;
    vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(
      () =>
        new Promise<void>((_resolve, fail) => {
          reject = fail;
        }),
    );
    await open(fake);
    fake.resolve();
    await waitFor(() => expect(screen.getByLabelText('录音播放控件')).toHaveAttribute('src'));
    const audio = metadata();
    Object.defineProperty(audio, 'currentSrc', { configurable: true, value: audio.src });
    await userEvent.click(screen.getByRole('button', { name: '播放录音' }));
    await userEvent.click(screen.getByRole('button', { name: '重新打开录音' }));
    await waitFor(() => expect(audio).not.toHaveAttribute('src'));
    await act(async () => reject(new Error('Old synthetic play rejection')));
    expect(screen.queryByText('Old synthetic play rejection')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: '播放录音' })).toBeDisabled();
  });
  it('shows a rejected mocked user play and disables the entry after a media error', async () => {
    const fake = mediaFixture();
    const play = vi
      .spyOn(HTMLMediaElement.prototype, 'play')
      .mockRejectedValue(new Error('Synthetic user play denied'));
    await open(fake);
    fake.resolve();
    await waitFor(() => expect(screen.getByLabelText('录音播放控件')).toHaveAttribute('src'));
    const audio = metadata();
    expect(play).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: '播放录音' }));
    expect(await screen.findByText('Synthetic user play denied')).toBeVisible();
    expect(play).toHaveBeenCalledTimes(1);
    fireEvent.error(audio);
    expect(screen.getByRole('button', { name: '播放录音' })).toBeDisabled();
    expect(screen.getByText(/录音解码失败或媒体授权已失效/)).toBeVisible();
  });
});
