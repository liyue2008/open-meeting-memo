// Synthetic archived audio through the production Tauri SDK/media path. No hardware playback.
(async () => {
  const checks = [], reads = [], media = [];
  let sequence = 0, realPlayCalls = 0, mockPlayCalls = 0, mockMode = false, maximumActive = 0;
  const active = new Set();
  const pause = HTMLMediaElement.prototype.pause;
  HTMLMediaElement.prototype.pause = function () { active.delete(this); return pause.call(this); };
  HTMLMediaElement.prototype.play = function () {
    if (!mockMode) { realPlayCalls++; throw Error('Hardware playback forbidden'); }
    mockPlayCalls++; active.add(this); maximumActive = Math.max(maximumActive, active.size);
    Object.defineProperty(this, 'paused', { configurable: true, get: () => !active.has(this) });
    this.dispatchEvent(new Event('play')); return Promise.resolve();
  };
  const check = (name, ok) => { if (!ok) throw Error(name); checks.push(name); };
  const raw = async (command, payload = {}) => {
    const value = await window.__TAURI_INTERNALS__.invoke(command, { request: { schemaVersion: 1, requestId: `archive-${++sequence}`, payload } });
    reads.push({ command, error: value.error?.code ?? null });
    if (value.error) throw Error(command + ': ' + JSON.stringify(value.error)); return value.data;
  };
  const wait = async (name, predicate) => {
    const until = Date.now() + 20000;
    do { const value = await predicate(); if (value) return value; await new Promise(r => setTimeout(r, 40)); } while (Date.now() < until);
    throw Error('Timeout: ' + name);
  };
  try {
    check('production trusted origin', location.origin === 'tauri://localhost');
    await wait('synthetic meeting', () => [...document.querySelectorAll('.meeting-title')].find(e => e.textContent === 'Private bridge fixture'));
    const id = '00000000-0000-4000-8000-000000000041';
    const manifest = await raw('media_manifest', { meetingId: id, cursor: null, pageSize: 100 });
    check('two genuine committed Opus segments', manifest.segments.length === 2 && manifest.segments.every(s => s.available && s.mime === 'audio/ogg'));
    for (const segment of manifest.segments) {
      const grant = await raw('media_open', { meetingId: id, segmentId: segment.segmentId, expectedRevision: manifest.revision });
      const response = await fetch(grant.url);
      const bytes = new Uint8Array(await response.arrayBuffer());
      const digest = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(v => v.toString(16).padStart(2, '0')).join('');
      check('actual audio/ogg response ' + segment.sequence, response.ok && response.headers.get('Content-Type').startsWith('audio/ogg'));
      check('Ogg bytes match native authorized manifest ' + segment.sequence, String.fromCharCode(...bytes.slice(0, 4)) === 'OggS' && digest === segment.sha256 && bytes.length === segment.byteLength);
      const range = await fetch(grant.url, { headers: { Range: 'bytes=0-43' } });
      const head = new Uint8Array(await range.arrayBuffer());
      check('valid actual protocol Range ' + segment.sequence, range.status === 206 && head.length === 44 && head.every((v, i) => v === bytes[i]));
      const invalid = await fetch(grant.url, { headers: { Range: 'bytes=999999999-1000000000' } });
      check('invalid actual protocol Range rejected ' + segment.sequence, invalid.status === 416);
      media.push({ sequence: segment.sequence, mime: response.headers.get('Content-Type'), hash: digest, bytes: bytes.length, validRange: range.status, invalidRange: invalid.status });
      await raw('media_close', { grantId: grant.grantId });
    }
    [...document.querySelectorAll('.meeting-title')].find(e => e.textContent === 'Private bridge fixture').click();
    const audio = await wait('real archived metadata', () => { const a = document.querySelector('audio'); return a?.readyState >= 1 && Number.isFinite(a.duration) && a.duration > 0 && a; });
    // WK reports 100 ms Opus container pre-roll; the archive codec separately verifies exact decoded frames.
    media.push({metadataDuration:audio.duration, timelineDurationMs:manifest.durationMs, paused:audio.paused, autoplay:audio.autoplay});
    check('native metadata remains paused without autoplay', audio.paused && !audio.autoplay && audio.duration > 2.9 && audio.duration <= 3.11);
    audio.currentTime = 1.2;
    await wait('paused native seek', () => audio.paused && Math.abs(audio.currentTime - 1.2) < .1);
    media.push({ metadataDuration: audio.duration, pausedSeek: audio.currentTime, paused: audio.paused });
    const oldUrl = audio.src;
    check('exactly one attached audio element', document.querySelectorAll('audio').length === 1);
    mockMode = true;
    const playback = await wait('visible custom playback button', () => [...document.querySelectorAll('button')].find(e => e.textContent.trim() === '播放录音' && !e.disabled));
    playback.click();
    await wait('single mock active instance', () => active.size === 1);
    const back = [...document.querySelectorAll('button')].find(e => e.textContent.trim() === '← 返回会议库');
    check('real navigation entry', !!back); back.click();
    await wait('old media released after navigation', () => !audio.isConnected && active.size === 0 && !audio.hasAttribute('src'));
    const revoked = await fetch(oldUrl);
    check('old real media authorization revoked', [403, 404, 410].includes(revoked.status));
    check('mock one active instance only; no hardware play', maximumActive === 1 && mockPlayCalls === 1 && realPlayCalls === 0);
    window.__ommFixtureResult = { ok: true, checks, reads, media, playCalls: realPlayCalls, mockPlayCalls, maximumActive, activeAfterNavigation: active.size, sourceCleared: !audio.hasAttribute('src'), detached: !audio.isConnected, syntheticOnly: true, schedulerStarted: false };
  } catch (error) {
    const a = document.querySelector('audio');
    window.__ommFixtureResult = { ok: false, checks, reads, media, playCalls: realPlayCalls, mockPlayCalls, error: String(error), currentMedia: a && {duration: Number.isFinite(a.duration) ? a.duration : String(a.duration), paused:a.paused, autoplay:a.autoplay, readyState:a.readyState, networkState:a.networkState, currentTime:a.currentTime, error:a.error?.code ?? null}, body: document.body.innerText.slice(0, 8000) };
  }
})();
