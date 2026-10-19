(async () => {
  const checks = []; let playCalls = 0;
  HTMLMediaElement.prototype.play = function () { playCalls++; throw Error('play forbidden'); };
  const check = (name, value) => { if (!value) throw Error(name); checks.push(name); };
  const raw = async (command, payload = {}) => {
    const r = await window.__TAURI_INTERNALS__.invoke(command, { request: { schemaVersion: 1, requestId: crypto.randomUUID(), payload } });
    if (r.error) throw Error(command + ':' + JSON.stringify(r.error)); return r.data;
  };
  const wait = async (name, predicate) => {
    const end = Date.now() + 20000;
    do { const r = await predicate(); if (r) return r; await new Promise(resolve => setTimeout(resolve, 50)); } while (Date.now() < end);
    throw Error('Timeout: ' + name);
  };
  const visible = e => e && e.getClientRects().length;
  const modal = () => [...document.querySelectorAll('dialog[open],[role=dialog]')].find(visible);
  const field = name => [...document.querySelectorAll('label')].find(e => e.childNodes[0]?.textContent.trim() === name)?.querySelector('input,select') || document.querySelector('[aria-label="' + name + '"]');
  const click = async (name, root = document) => {
    const b = await wait(name, () => [...root.querySelectorAll('button,a')].find(e => visible(e) && !e.disabled && (e.textContent.trim() === name || e.getAttribute('aria-label') === name)));
    b.click(); await new Promise(r => setTimeout(r, 50));
  };
  const set = async (element, value) => {
    Object.getOwnPropertyDescriptor(element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype, 'value').set.call(element, value);
    element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
  };
  const close = async (id, action, result) => {
    window.__ommFixtureCloseResult = null; window.__ommFixtureCloseRequest = { id, action, ...(result ? { result } : {}) };
    const reply = await wait('owned native close result ' + id, () => window.__ommFixtureCloseResult?.id === id && window.__ommFixtureCloseResult);
    check('native close admitted: ' + id, reply.ok && (action === 'close' ? reply.nativeCloseRequested : reply.armed && reply.mainAlive));
  };
  try {
    check('phase is explicit', ['write', 'read'].includes(window.__ommPreferencePhase));
    const store = await wait('native actual UUID attachment', () => window.__ommOwnedStoreAttached);
    check('actual persistent owned store verified before access', store.actualPersistent && store.ownedMarkerVerified && typeof store.actualIdentifier === 'string');
    await wait('actual production React appearance control', () => field('外观主题'));
    const initial = { theme: document.documentElement.dataset.theme, sidebar: document.documentElement.dataset.sidebar, preference: field('外观主题').value, storedTheme: localStorage.getItem('omm.theme'), storedSidebar: localStorage.getItem('omm.sidebar') };
    if (window.__ommPreferencePhase === 'read') {
      check('second process initially restores dark before any preference write', initial.theme === 'dark' && initial.preference === 'dark' && initial.storedTheme === 'dark');
      check('second process initially restores collapsed sidebar', initial.sidebar === 'collapsed' && initial.storedSidebar === 'collapsed');
    } else {
      check('fresh owned pair has no inherited preferences', initial.storedTheme === null && initial.storedSidebar === null);
      await set(field('外观主题'), 'dark'); await wait('real dark render', () => document.documentElement.dataset.theme === 'dark');
      await click('收起侧栏'); await wait('real collapsed render', () => document.documentElement.dataset.sidebar === 'collapsed');
    }
    check('stored only UI preferences and verified ownership marker', [...Array(localStorage.length)].map((_, i) => localStorage.key(i)).sort().join('|') === ['__omm_owned_wk_nonce','omm.sidebar','omm.theme'].sort().join('|'));
    await click('设置'); await wait('real configuration page', () => field('自动录音开始延迟（毫秒）'));
    const before = await raw('config_get'); await set(field('自动录音开始延迟（毫秒）'), String(before.values.startDelayMs + 19));
    await wait('real dirty draft', () => document.body.innerText.includes('有未保存的修改'));
    await close('preferences-cancel', 'close'); await wait('real close prompt', () => modal()?.textContent.includes('设置尚未保存'));
    await click('留在设置', modal()); await wait('real cancel retains main and draft', () => !modal() && field('自动录音开始延迟（毫秒）').value === String(before.values.startDelayMs + 19));
    check('cancel leaves configuration untouched', (await raw('config_get')).revision === before.revision);
    await close('preferences-confirm', 'close'); await wait('real second prompt', () => modal()?.textContent.includes('设置尚未保存'));
    const result = { ok: true, checks, playCalls, initial, store, phase: window.__ommPreferencePhase, configRevision: before.revision };
    await close('preferences-finish', 'arm-destroy', result); await click('放弃并离开', modal());
  } catch (error) { window.__ommFixtureResult = { ok: false, checks, playCalls, error: String(error), body: document.body.innerText.slice(0, 10000) }; }
})();
