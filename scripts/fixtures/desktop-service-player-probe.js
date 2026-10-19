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
  let titlebarInset;
  const shot = async request => {
    await stable(); window.__ommFixtureSnapshotResult = null;
    window.__ommFixtureSnapshotRequest = request;
    const value = await wait(request.name, () => window.__ommFixtureSnapshotResult);
    check('stable actual WK snapshot ' + request.name, value.ok && value.name === request.name && value.bytes > 0);
    return value;
  };
  const snapshot = async (name, width, height) => {
    if (titlebarInset === undefined) {
      await shot({name:'service-viewport-calibration',width,height}); titlebarInset = height - innerHeight;
      check('measured actual service titlebar inset', titlebarInset >= 0 && titlebarInset <= 100);
    }
    await shot({name:name + '-size',width,height:height + titlebarInset});
    check('actual service content dimensions ' + name, innerWidth === width && innerHeight === height);
    const value = await shot({name}); snapshots.push({...value,viewport:[innerWidth,innerHeight],titlebarInset});
    check('no service page horizontal overflow ' + name, document.documentElement.scrollWidth <= innerWidth + 1);
  };
  const rgba = color => { const values = color.match(/[\d.]+/g).map(Number); return [...values.slice(0,3),values[3] ?? 1]; };
  const background = element => {
    const ancestors = []; for (let node = element; node; node = node.parentElement) ancestors.unshift(node);
    return ancestors.reduce((base,node) => { const color = rgba(getComputedStyle(node).backgroundColor); return base.map((v,i) => color[i]*color[3] + v*(1-color[3])); },[255,255,255]);
  };
  const luminance = rgb => rgb.map(v => v / 255).map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4).reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0);
  const contrast = (foreground, background) => { const x = luminance(rgba(foreground).slice(0,3)), y = luminance(background); return (Math.max(x, y) + .05) / (Math.min(x, y) + .05); };
  const field = name => [...document.querySelectorAll('label')].find(e => e.textContent.trim() === name)?.htmlFor;
  const searchField = () => document.querySelector('input[aria-label="搜索会议"]') || document.getElementById(field('搜索会议'));
  try {
    const banner = await wait('real unavailable service banner', () => document.querySelector('.compatibility-banner'));
    const entry = banner.querySelector('.service-entry');
    check('scoped actual service entry', entry && /打开.*设置/.test(entry.textContent));
    for (const theme of ['light', 'dark']) {
      entry.blur();
      change(document.querySelector('select[aria-label="外观主题"]'), theme);
      await wait('actual theme ' + theme, () => document.documentElement.dataset.theme === theme); await stable();
      const messageNode = banner.querySelector('.mantine-Alert-message') || banner;
      const button = getComputedStyle(entry), message = getComputedStyle(messageNode);
      const evidence = { theme, foreground: button.color, background: button.backgroundColor, resolvedBackground:background(entry), appearance: button.getPropertyValue('-webkit-appearance'), buttonContrast: contrast(button.color, background(entry)), bannerForeground:message.color, bannerBackground:background(messageNode), bannerContrast: contrast(message.color, background(messageNode)), message: banner.innerText };
      styles.push(evidence);
      check('service button normal contrast >=4.5 ' + theme, evidence.buttonContrast >= 4.5);
      check('service message contrast >=4.5 ' + theme, evidence.bannerContrast >= 4.5);
      check('service entry disables native paint ' + theme, evidence.appearance === 'none');
      await snapshot('service-entry-' + theme, 800, 600);
      entry.focus({ preventScroll: true });
      const focus = getComputedStyle(entry);
      evidence.focusContrast = contrast(focus.color, background(entry));
      check('actual focus state contrast >=4.5 ' + theme, document.activeElement === entry && evidence.focusContrast >= 4.5);
    }
    const search = await wait('real library query field', searchField);
    check('real library query field', search);
    change(search, 'Private bridge fixture');
    await wait('actual query result settles', () => document.querySelectorAll('[aria-label="会议列表"] tbody tr').length === 1);
    check('actual query retains expected synthetic meeting', document.querySelector('[aria-label="会议列表"] tbody')?.textContent.includes('Private bridge fixture'));
    const meeting = await wait('actual synthetic detail entry', () => [...document.querySelectorAll('.meeting-title')].find(e => e.textContent === 'Private bridge fixture'));
    meeting.click();
    const audio = await wait('actual ready paused audio', () => { const a = document.querySelector('audio'); return a?.readyState >= 1 && a; });
    const playback = await wait('enabled visible user playback entry', () => [...document.querySelectorAll('button')].find(e => e.textContent.trim() === '播放录音' && !e.disabled && e.getClientRects().length));
    check('single hidden audio without native controls, metadata preload and no autoplay', document.querySelectorAll('audio').length === 1 && audio.hidden && !audio.controls && !audio.autoplay && audio.preload === 'metadata');
    check('visible named playback button controls actual audio', playback.getAttribute('aria-controls') === audio.id && playback.tabIndex === 0);
    check('metadata is finite and remains paused', Number.isFinite(audio.duration) && audio.paused);
    const playbackStyle = getComputedStyle(playback);
    check('visible playback entry contrast >=4.5', contrast(playbackStyle.color, background(playback)) >= 4.5);
    const audioEvidence = { readyState: audio.readyState, duration: audio.duration, currentTime: audio.currentTime, paused: audio.paused, controls: audio.controls, buttonText: playback.textContent.trim(), buttonBounds: playback.getBoundingClientRect().toJSON(), audioId: audio.id };
    playback.scrollIntoView({block:'center'});
    await snapshot('playback-entry-paused', 1280, 820);
    check('no playback requested', playCalls === 0);
    const back = [...document.querySelectorAll('button')].find(e => e.textContent.trim() === '← 返回会议库');
    check('actual media navigation cleanup entry', !!back); back.click();
    await wait('old hidden audio released before fixture completion', () => !audio.isConnected && !audio.hasAttribute('src'));
    audioEvidence.sourceClearedBeforeTeardown = true;
    window.__ommFixtureResult = { ok: true, checks, playCalls, styles, audioEvidence, snapshots, hoverPointerTested: false, hoverEvidence: 'Computed normal/focus contrast observed; no physical hover claimed.' };
  } catch (error) {
    window.__ommFixtureResult = { ok: false, checks, playCalls, styles, snapshots, error: String(error), body: document.body.innerText.slice(0, 12000) };
  }
})();
