import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { describe, expect, it, vi } from 'vitest';
import { BridgeContext } from '../src/app/context';
import { Player } from '../src/features/player';
import type { DesktopCommandMap } from '../src/contracts.generated';
import { fixtureBridge, sample } from './fixtures';

const firstMeeting = '00000000-0000-4000-8000-000000000001';
const secondMeeting = '00000000-0000-4000-8000-000000000002';
function fixture() {
  const opens: ((grant: DesktopCommandMap['media_open']['response']) => void)[] = [];
  let revision = 'a'.repeat(64);
  const openedRevisions: string[] = [];
  const fake = fixtureBridge({
    media_manifest: (body) => ({
      ...sample('media_manifest'),
      revision,
      meetingId: body.meetingId,
      durationMs: 20000,
      segments: [0, 1].map((sequence) => ({
        segmentId: `00000000-0000-4000-8000-00000000001${sequence}`,
        sequence,
        offsetMs: sequence * 10000,
        durationMs: 10000,
        mime: 'audio/wav',
        byteLength: 320044,
        sha256: 'a'.repeat(64),
        available: true,
        reason: null,
      })),
    }),
    media_open: (body) => {
      openedRevisions.push(body.expectedRevision);
      return new Promise((resolve) => opens.push(resolve));
    },
  });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  function tree(meetingId = firstMeeting, shown = true) {
    return (
      <QueryClientProvider client={client}>
        <BridgeContext.Provider value={fake.bridge}>
          {shown && <Player meetingId={meetingId} seek={null} />}
        </BridgeContext.Provider>
      </QueryClientProvider>
    );
  }
  const view = render(tree());
  async function grant(index: number) {
    await waitFor(() => expect(opens.length).toBeGreaterThan(index));
    await act(async () =>
      opens[index]!({
        ...sample('media_open'),
        revision: openedRevisions[index]!,
        grantId: `lifecycle_grant_${index.toString().padStart(16, '0')}`,
        url: `omm-media://localhost/lifecycle_grant_${index.toString().padStart(16, '0')}`,
      }),
    );
  }
  async function ready(index: number) {
    await grant(index);
    const element = screen.getByLabelText('录音播放控件') as HTMLAudioElement;
    Object.defineProperty(element, 'duration', { configurable: true, value: 10 });
    Object.defineProperty(element, 'readyState', { configurable: true, value: 1 });
    fireEvent.loadedMetadata(element);
    return element;
  }
  return {
    fake,
    view,
    tree,
    grant,
    ready,
    opens,
    client,
    setRevision: (value: string) => {
      revision = value;
    },
  };
}
function expectReleased(
  element: HTMLAudioElement,
  pause: ReturnType<typeof vi.spyOn>,
  load: ReturnType<typeof vi.spyOn>,
) {
  expect(pause.mock.instances).toContain(element);
  expect(element).not.toHaveAttribute('src');
  expect(load.mock.instances).toContain(element);
}

describe('features/player lifecycle (mock media only, never hardware playback)', () => {
  it('releases browser bytes on leave and reopens a single paused element', async () => {
    const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause');
    const load = vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => undefined);
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue();
    const f = fixture();
    const old = await f.ready(0);
    fireEvent.click(screen.getByRole('button', { name: '播放录音' }));
    expect(play).toHaveBeenCalledTimes(1);
    f.view.rerender(f.tree(firstMeeting, false));
    expectReleased(old, pause, load);
    f.view.rerender(f.tree());
    const current = await f.ready(1);
    expect(document.querySelectorAll('audio')).toHaveLength(1);
    expect(current).not.toBe(old);
    expect(play).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(f.fake.calls.filter((c) => c.command === 'media_close')).toHaveLength(1),
    );
  });
  it('releases old elements on segment, reload and meeting changes; stale play cannot touch the new element', async () => {
    const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause');
    const load = vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => undefined);
    let resolve!: () => void;
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(
      () =>
        new Promise<void>((r) => {
          resolve = r;
        }),
    );
    const f = fixture();
    const old = await f.ready(0);
    fireEvent.click(screen.getByRole('button', { name: '播放录音' }));
    fireEvent.click(screen.getByRole('button', { name: '播放录音' }));
    expect(play).toHaveBeenCalledTimes(1);
    fireEvent.change(screen.getByLabelText('录音分段'), {
      target: { value: '00000000-0000-4000-8000-000000000011' },
    });
    const second = await f.ready(1);
    expectReleased(old, pause, load);
    expect(second).not.toBe(old);
    const newPauses = pause.mock.instances.filter((x) => x === second).length;
    await act(async () => resolve());
    expect(pause.mock.instances.filter((x) => x === second)).toHaveLength(newPauses);
    fireEvent.click(screen.getByRole('button', { name: '重新打开录音' }));
    const reloaded = await f.ready(2);
    expectReleased(second, pause, load);
    f.view.rerender(f.tree(secondMeeting));
    await f.ready(3);
    expectReleased(reloaded, pause, load);
    expect(f.fake.calls.filter((c) => c.command === 'media_open').at(-1)?.payload).toMatchObject({
      meetingId: secondMeeting,
      segmentId: '00000000-0000-4000-8000-000000000010',
    });
    expect(play).toHaveBeenCalledTimes(1);
  });
  it('releases a replaced manifest revision and ignores its late play rejection', async () => {
    const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause');
    const load = vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => undefined);
    let reject!: (reason: Error) => void;
    vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(
      () =>
        new Promise<void>((_, r) => {
          reject = r;
        }),
    );
    const f = fixture();
    const old = await f.ready(0);
    fireEvent.click(screen.getByRole('button', { name: '播放录音' }));
    f.setRevision('b'.repeat(64));
    await act(async () => {
      await f.client.refetchQueries({ queryKey: ['media_manifest', firstMeeting] });
    });
    const current = await f.ready(1);
    expectReleased(old, pause, load);
    expect(current).toHaveAttribute('src');
    const newPauses = pause.mock.instances.filter((x) => x === current).length;
    await act(async () => reject(new Error('synthetic obsolete play rejection')));
    expect(screen.queryByText('synthetic obsolete play rejection')).not.toBeInTheDocument();
    expect(current).toHaveAttribute('src');
    expect(pause.mock.instances.filter((x) => x === current)).toHaveLength(newPauses);
  });
  it('keeps an unmounted play completion attached only to the discarded element', async () => {
    const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause');
    const load = vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => undefined);
    let resolve!: () => void;
    vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(
      () =>
        new Promise<void>((r) => {
          resolve = r;
        }),
    );
    const f = fixture();
    const old = await f.ready(0);
    fireEvent.click(screen.getByRole('button', { name: '播放录音' }));
    f.view.unmount();
    expectReleased(old, pause, load);
    await act(async () => resolve());
    expectReleased(old, pause, load);
    expect(document.querySelectorAll('audio')).toHaveLength(0);
  });
  it('closes out-of-order grants once, including a grant delivered after unmount', async () => {
    vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => undefined);
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue();
    const f = fixture();
    await waitFor(() => expect(f.opens).toHaveLength(1));
    fireEvent.click(screen.getByRole('button', { name: '重新打开录音' }));
    await f.ready(1);
    const current = screen.getByLabelText('录音播放控件');
    const source = current.getAttribute('src');
    await f.grant(0);
    expect(current).toHaveAttribute('src', source);
    fireEvent.click(screen.getByRole('button', { name: '重新打开录音' }));
    await waitFor(() => expect(f.opens).toHaveLength(3));
    f.view.unmount();
    await f.grant(2);
    await waitFor(() =>
      expect(f.fake.calls.filter((c) => c.command === 'media_close')).toHaveLength(3),
    );
    expect(play).not.toHaveBeenCalled();
  });
});
