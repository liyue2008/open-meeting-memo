import { Group, Slider, Button, NativeSelect, Paper, Text, Title } from '@mantine/core';
import { useEffect, useRef, useState } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import { useBridge } from '../app/context';
import { clock } from '../app/dates';
import { ErrorNotice, Loading } from '../components/ui';
import type { MediaSegment } from '../contracts.generated';

export function segmentAt(segments: MediaSegment[], position: number): MediaSegment | undefined {
  return (
    segments.find(
      (s) => s.available && position >= s.offsetMs && position < s.offsetMs + s.durationMs,
    ) ?? segments.find((s) => s.available && position === s.offsetMs + s.durationMs)
  );
}
type PlayerProps = {
  meetingId: string;
  seek: { time: number; serial: number } | null;
};
export function Player(props: PlayerProps) {
  return <MeetingPlayer key={props.meetingId} {...props} />;
}
function releaseMedia(element: HTMLAudioElement) {
  element.pause();
  element.removeAttribute('src');
  element.load();
}
function MeetingPlayer({ meetingId, seek }: PlayerProps) {
  const bridge = useBridge();
  const audio = useRef<HTMLAudioElement>(null);
  const [segment, setSegment] = useState<MediaSegment | null>(null);
  const [source, setSource] = useState<{ key: string; url: string } | null>(null);
  const [position, setPosition] = useState(0);
  const [rate, setRate] = useState(1);
  const [error, setError] = useState<unknown>(null);
  const [opening, setOpening] = useState(false);
  const [ready, setReady] = useState(false);
  const [playing, setPlaying] = useState(false);
  const pendingSeek = useRef(0);
  const pendingPlay = useRef<HTMLAudioElement | null>(null);
  const [reload, setReload] = useState(0);
  const manifest = useInfiniteQuery({
    queryKey: ['media_manifest', meetingId],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) =>
      bridge.call('media_manifest', { meetingId, cursor: pageParam, pageSize: 100 }, signal),
    getNextPageParam: (p) => p.nextCursor ?? undefined,
  });
  const segments = manifest.data?.pages.flatMap((p) => p.segments) ?? [];
  const revision = manifest.data?.pages[0]?.revision;
  const duration = manifest.data?.pages[0]?.durationMs ?? 0;
  const sourceKey = `${segment?.segmentId}:${revision}:${reload}`;
  const url = source?.key === sourceKey ? source.url : '';
  function locate(time: number) {
    const bounded = Math.max(0, Math.min(duration, time));
    const target = segmentAt(segments, bounded);
    if (!target) {
      setError(new Error('此时间位于录音间隙、不可用分段或尚未读取的分段。'));
      return;
    }
    setError(null);
    pendingSeek.current =
      Math.max(0, Math.min(target.durationMs, bounded - target.offsetMs)) / 1000;
    setPosition(bounded);
    if (target.segmentId === segment?.segmentId && audio.current?.readyState)
      audio.current.currentTime = pendingSeek.current;
    else {
      audio.current?.pause();
      setSegment(target);
    }
  }
  useEffect(() => {
    if (seek) locate(seek.time);
  }, [seek?.serial]); // Explicit locator events, not every timeline tick.
  useEffect(() => {
    if (!segment && segments.length) {
      const first = segments.find((s) => s.available);
      if (first) {
        pendingSeek.current = 0;
        setSegment(first);
        setPosition(first.offsetMs);
      }
    }
  }, [manifest.data]);
  useEffect(() => {
    if (!segment || !revision) return;
    let disposed = false;
    let grantId: string | null = null;
    // Capture this source's element: cleanup must never clear its replacement.
    const element = audio.current;
    setSource(null);
    setReady(false);
    setPlaying(false);
    setOpening(true);
    setError(null);
    void bridge
      .call('media_open', { meetingId, segmentId: segment.segmentId, expectedRevision: revision })
      .then((grant) => {
        grantId = grant.grantId;
        if (disposed) {
          void bridge.call('media_close', { grantId }).catch(() => undefined);
          return;
        }
        setSource({ key: sourceKey, url: grant.url });
        setOpening(false);
      })
      .catch((e) => {
        if (!disposed) {
          setError(e);
          setOpening(false);
        }
      });
    return () => {
      disposed = true;
      // Closing a grant cannot revoke bytes already buffered by WebKit.
      if (element) releaseMedia(element);
      if (grantId) void bridge.call('media_close', { grantId }).catch(() => undefined);
    };
  }, [bridge, meetingId, segment?.segmentId, revision, reload]);
  useEffect(() => {
    const element = audio.current;
    if (element) element.playbackRate = rate;
  }, [rate, url]);
  function togglePlayback() {
    const element = audio.current;
    if (!element || opening || !ready || !url) return;
    if (!element.paused && !element.ended) {
      element.pause();
      return;
    }
    if (pendingPlay.current === element) return;
    const source = element.getAttribute('src');
    const isCurrent = () => audio.current === element && element.getAttribute('src') === source;
    pendingPlay.current = element;
    // Only this explicit user action requests playback; locating/reopening never does.
    void element
      .play()
      .then(
        () => {
          if (!isCurrent()) releaseMedia(element);
        },
        (failure: unknown) => {
          if (isCurrent()) setError(failure);
        },
      )
      .finally(() => {
        if (pendingPlay.current === element) pendingPlay.current = null;
      });
  }
  if (manifest.isPending)
    return (
      <Paper className="audio-player panel" withBorder p="md" component="section">
        <Loading />
      </Paper>
    );
  return (
    <Paper
      className="audio-player panel"
      aria-label="会议录音"
      withBorder
      p="md"
      component="section"
    >
      <div className="player-header">
        <Title order={2}>会议录音</Title>
        <Text component="span" size="sm" c="dimmed">
          公开混音 · 仅手动播放
        </Text>
      </div>
      {manifest.error && (
        <ErrorNotice error={manifest.error} retry={() => void manifest.refetch()} />
      )}
      {segments.length === 0 && (
        <Text className="empty-state" component="p">
          暂无可用的已归档公开混音。不会读取内部声源或无来源凭证的 WAV。
        </Text>
      )}
      {segments.length > 0 && (
        <>
          <audio
            key={sourceKey}
            ref={audio}
            id="meeting-recording-audio"
            hidden
            preload="metadata"
            src={url || undefined}
            aria-label="录音播放控件"
            onLoadedMetadata={() => {
              const el = audio.current;
              if (el) {
                setReady(true);
                el.playbackRate = rate;
                el.currentTime = Math.min(
                  pendingSeek.current,
                  Number.isFinite(el.duration) ? el.duration : pendingSeek.current,
                );
              }
            }}
            onPlay={() => setPlaying(true)}
            onPause={() => setPlaying(false)}
            onEnded={() => setPlaying(false)}
            onEmptied={() => {
              setReady(false);
              setPlaying(false);
            }}
            onTimeUpdate={() => {
              if (audio.current && segment)
                setPosition(segment.offsetMs + audio.current.currentTime * 1000);
            }}
            onError={() => {
              setReady(false);
              setPlaying(false);
              if (url) setError(new Error('录音解码失败或媒体授权已失效，请重新打开。'));
            }}
          />
          <Group gap="sm">
            <Button
              type="button"
              className="player-playback"
              disabled={opening || !url || !ready}
              onClick={togglePlayback}
              aria-controls="meeting-recording-audio"
              variant="default"
            >
              {playing ? '暂停录音' : '播放录音'}
            </Button>
            <Text size="sm">
              会议全局时间 {clock(position)} / {clock(duration)}
            </Text>
          </Group>
          <div className="timeline-label">
            <Slider
              thumbLabel="会议时间定位"
              thumbValueText={clock}
              thumbProps={{ 'aria-describedby': 'meeting-recording-timeline-help' }}
              label={clock}
              min={0}
              max={duration}
              step={100}
              value={position}
              onChange={locate}
            />
          </div>
          <div className="button-row">
            <Button
              disabled={opening}
              onClick={() => locate(position - 15000)}
              type="button"
              variant="default"
            >
              后退 15 秒
            </Button>
            <Button
              disabled={opening}
              onClick={() => locate(position + 15000)}
              type="button"
              variant="default"
            >
              前进 15 秒
            </Button>
            <label>
              速度
              <NativeSelect value={rate} onChange={(e) => setRate(Number(e.target.value))}>
                {[0.75, 1, 1.25, 1.5, 2].map((v) => (
                  <option key={v} value={v}>
                    {v}×
                  </option>
                ))}
              </NativeSelect>
            </label>
            <label>
              当前分段
              <NativeSelect
                aria-label="录音分段"
                value={segment?.segmentId ?? ''}
                onChange={(e) => {
                  const target = segments.find((s) => s.segmentId === e.target.value);
                  if (target) locate(target.offsetMs);
                }}
              >
                {segments.map((s) => (
                  <option key={s.segmentId} value={s.segmentId} disabled={!s.available}>
                    第 {s.sequence + 1} 段 · {clock(s.offsetMs)}–{clock(s.offsetMs + s.durationMs)}
                    {!s.available ? ' · 不可用' : ''}
                  </option>
                ))}
              </NativeSelect>
            </label>
            <Button
              onClick={() => {
                audio.current?.pause();
                setReload((n) => n + 1);
              }}
              type="button"
              variant="default"
            >
              重新打开录音
            </Button>
          </div>
          <Text
            id="meeting-recording-timeline-help"
            className="muted"
            component="p"
            size="xs"
            c="dimmed"
          >
            左右方向键微调，Home / End 跳至首尾。跨段定位保持暂停，录音间隙保留。
          </Text>
          {opening && (
            <Text role="status" component="p">
              正在验证录音…
            </Text>
          )}
        </>
      )}
      {!!error && <ErrorNotice error={error} />}{' '}
      {manifest.hasNextPage && (
        <Button
          disabled={manifest.isFetchingNextPage}
          onClick={() => void manifest.fetchNextPage()}
          type="button"
          variant="default"
        >
          读取更多录音分段
        </Button>
      )}
    </Paper>
  );
}
