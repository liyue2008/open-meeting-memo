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
export function Player({
  meetingId,
  seek,
}: {
  meetingId: string;
  seek: { time: number; serial: number } | null;
}) {
  const bridge = useBridge();
  const audio = useRef<HTMLAudioElement>(null);
  const [segment, setSegment] = useState<MediaSegment | null>(null);
  const [url, setUrl] = useState('');
  const [position, setPosition] = useState(0);
  const [rate, setRate] = useState(1);
  const [error, setError] = useState<unknown>(null);
  const [opening, setOpening] = useState(false);
  const pendingSeek = useRef(0);
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
    setUrl('');
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
        setUrl(grant.url);
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
      if (grantId) void bridge.call('media_close', { grantId }).catch(() => undefined);
    };
  }, [bridge, meetingId, segment?.segmentId, revision, reload]);
  useEffect(() => {
    const element = audio.current;
    if (element) element.playbackRate = rate;
  }, [rate, url]);
  if (manifest.isPending)
    return (
      <section className="audio-player panel">
        <Loading />
      </section>
    );
  return (
    <section className="audio-player panel" aria-label="会议录音">
      <div className="player-header">
        <h2>会议录音</h2>
        <span className="muted">已验证的公开混音 · 点击原生播放键才会发声</span>
      </div>
      {manifest.error && (
        <ErrorNotice error={manifest.error} retry={() => void manifest.refetch()} />
      )}
      {segments.length === 0 && (
        <p className="empty-state">
          暂无可用的已归档公开混音。不会读取内部声源或无来源凭证的 WAV。
        </p>
      )}
      {segments.length > 0 && (
        <>
          <audio
            ref={audio}
            controls
            preload="metadata"
            src={url || undefined}
            aria-label="录音播放控件"
            onLoadedMetadata={() => {
              const el = audio.current;
              if (el) {
                el.playbackRate = rate;
                el.currentTime = Math.min(
                  pendingSeek.current,
                  Number.isFinite(el.duration) ? el.duration : pendingSeek.current,
                );
              }
            }}
            onTimeUpdate={() => {
              if (audio.current && segment)
                setPosition(segment.offsetMs + audio.current.currentTime * 1000);
            }}
            onError={() => {
              if (url) setError(new Error('录音解码失败或媒体授权已失效，请重新打开。'));
            }}
          />
          <label className="timeline-label">
            会议时间 {clock(position)} / {clock(duration)}
            <input
              type="range"
              aria-label="会议时间定位"
              min="0"
              max={duration}
              step="100"
              value={position}
              onChange={(e) => locate(Number(e.target.value))}
            />
          </label>
          <div className="button-row">
            <button disabled={opening} onClick={() => locate(position - 15000)}>
              后退 15 秒
            </button>
            <button disabled={opening} onClick={() => locate(position + 15000)}>
              前进 15 秒
            </button>
            <label>
              速度
              <select value={rate} onChange={(e) => setRate(Number(e.target.value))}>
                {[0.75, 1, 1.25, 1.5, 2].map((v) => (
                  <option key={v} value={v}>
                    {v}×
                  </option>
                ))}
              </select>
            </label>
            <label>
              分段
              <select
                aria-label="录音分段"
                value={segment?.segmentId ?? ''}
                onChange={(e) => {
                  const target = segments.find((s) => s.segmentId === e.target.value);
                  if (target) locate(target.offsetMs);
                }}
              >
                {segments.map((s) => (
                  <option key={s.segmentId} value={s.segmentId} disabled={!s.available}>
                    第 {s.sequence + 1} 段 · {clock(s.offsetMs)}
                    {!s.available ? ' · 不可用' : ''}
                  </option>
                ))}
              </select>
            </label>
            <button
              onClick={() => {
                audio.current?.pause();
                setReload((n) => n + 1);
              }}
            >
              重新打开录音
            </button>
          </div>
          <p className="muted">跨段定位保持暂停，不自动播放。录音空隙保留；时间轴不是测量波形。</p>
          {opening && <p role="status">正在验证录音…</p>}
        </>
      )}
      {!!error && <ErrorNotice error={error} />}{' '}
      {manifest.hasNextPage && (
        <button
          disabled={manifest.isFetchingNextPage}
          onClick={() => void manifest.fetchNextPage()}
        >
          读取更多录音分段
        </button>
      )}
    </section>
  );
}
