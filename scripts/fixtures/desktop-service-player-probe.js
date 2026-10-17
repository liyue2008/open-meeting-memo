// Actual scoped CSS, native media metadata and visible user entry; never request playback.
(async () => {
  const checks = [], styles = [], snapshots = [];
  let playCalls = 0;
  HTMLMediaElement.prototype.play = () => { playCalls++; throw Error('Playback forbidden'); };
  const check = (name, value) => { if (!value) throw Error(name); checks.push(name); };
  const wait = async (name, predicate) => {
    const end = Date.now() + 20000;
    do { const value = predicate(); if (value) return value; await new Promise(r => setTimeout(r, 40)); } while (Date.now() < end);
    throw Error('Timeout: ' + name);
  };
  const change = (element, value) => {
    const proto = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(element, value);
    element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
  };
  // requestAnimationFrame may be suspended for an occluded WK window; bound stabilization.
  const stable = async () => { await Promise.race([document.fonts.ready, new Promise(r => setTimeout(r, 1500))]); await new Promise(r => setTimeout(r, 300)); };
  const snapshot = async (name, width, height) => {
    await stable(); window.__ommFixtureSnapshotResult = null;
    window.__ommFixtureSnapshotRequest = { name, width, height };
    const value = await wait(name, () => window.__ommFixtureSnapshotResult);
    check('stable actual WK snapshot ' + name, value.ok && value.name === name && value.bytes > 0); snapshots.push(value);
  };
  const rgb = color => color.match(/[\d.]+/g).slice(0, 3).map(Number);
  const luminance = color => rgb(color).map(v => v / 255).map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4).reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0);
  const contrast = (a, b) => { const x = luminance(a), y = luminance(b); return (Math.max(x, y) + .05) / (Math.min(x, y) + .05); };
  try {
    const banner = await wait('real unknown service banner', () => document.querySelector('.compatibility-banner'));
    const entry = banner.querySelector('.service-entry');
    check('scoped actual service entry', entry && /打开.*设置/.test(entry.textContent));
    for (const theme of ['light', 'dark']) {
      entry.blur();
      change(document.querySelector('select[aria-label="外观主题"]'), theme);
      await wait('actual theme ' + theme, () => document.documentElement.dataset.theme === theme); await stable();
      const button = getComputedStyle(entry), message = getComputedStyle(banner);
      const evidence = { theme, foreground: button.color, background: button.backgroundColor, appearance: button.getPropertyValue('-webkit-appearance'), buttonContrast: contrast(button.color, button.backgroundColor), bannerContrast: contrast(message.color, message.backgroundColor), message: banner.innerText };
      styles.push(evidence);
      check('service button normal contrast >=4.5 ' + theme, evidence.buttonContrast >= 4.5);
      check('service message contrast >=4.5 ' + theme, evidence.bannerContrast >= 4.5);
      check('service entry disables native paint ' + theme, evidence.appearance === 'none');
      await snapshot('service-entry-' + theme, 640, 960);
      entry.focus({ preventScroll: true });
      const focus = getComputedStyle(entry);
      evidence.focusContrast = contrast(focus.color, focus.backgroundColor);
      check('actual focus state contrast >=4.5 ' + theme, document.activeElement === entry && evidence.focusContrast >= 4.5);
    }
    const search = await wait('real library query field', () => document.querySelector('input[aria-label="搜索会议"]'));
    check('real library query field', search);
    change(search, 'Private bridge fixture');
    await wait('actual query result settles', () => document.querySelectorAll('[aria-label="会议列表"] tbody tr').length === 1);
    check('actual query retains expected synthetic meeting', document.querySelector('[aria-label="会议列表"] tbody')?.textContent.includes('Private bridge fixture'));
    const meeting = await wait('actual synthetic detail entry', () => [...document.querySelectorAll('.meeting-title')].find(e => e.textContent === 'Private bridge fixture'));
    meeting.click();
    const audio = await wait('actual ready paused audio', () => { const a = document.querySelector('audio'); return a?.readyState >= 1 && a; });
    const playback = await wait('enabled visible user playback entry', () => [...document.querySelectorAll('button')].find(e => e.textContent.trim() === '播放录音' && !e.disabled && e.getClientRects().length));
    check('native controls kept and autoplay absent', audio.controls && !audio.autoplay && audio.preload === 'metadata');
    check('visible named playback button controls actual audio', playback.getAttribute('aria-controls') === audio.id && playback.tabIndex === 0);
    check('metadata is finite and remains paused', Number.isFinite(audio.duration) && audio.paused);
    const playbackStyle = getComputedStyle(playback);
    check('visible playback entry contrast >=4.5', contrast(playbackStyle.color, playbackStyle.backgroundColor) >= 4.5);
    const audioEvidence = { readyState: audio.readyState, duration: audio.duration, currentTime: audio.currentTime, paused: audio.paused, controls: audio.controls, buttonText: playback.textContent.trim(), buttonBounds: playback.getBoundingClientRect().toJSON(), audioId: audio.id };
    await snapshot('playback-entry-paused', 1280, 1000);
    check('no playback requested', playCalls === 0);
    window.__ommFixtureResult = { ok: true, checks, playCalls, styles, audioEvidence, snapshots, hoverPointerTested: false, hoverEvidence: 'Scoped shared normal/hover/focus declarations verified by source-named unit test; no simulated hover claimed.' };
  } catch (error) {
    window.__ommFixtureResult = { ok: false, checks, playCalls, styles, snapshots, error: String(error), body: document.body.innerText.slice(0, 12000) };
  }
})();
