// Actual React + production Tauri transport/core, with only the private system-manager boundary faked.
(async () => {
  const checks = [], snapshots = [], layouts = [], operations = [], cspViolations = [];
  document.addEventListener('securitypolicyviolation', event => cspViolations.push({directive:event.effectiveDirective, blocked:event.blockedURI}));
  let playCalls = 0;
  HTMLMediaElement.prototype.play = () => { playCalls++; throw Error('Playback forbidden'); };
  const check = (name, value) => { if (!value) throw Error(name); checks.push(name); };
  const wait = async (name, predicate, budget = 30000) => {
    const end = Date.now() + budget;
    do { const value = await predicate(); if (value) return value; await new Promise(r => setTimeout(r, 80)); } while (Date.now() < end);
    throw Error('Timeout: ' + name);
  };
  const raw = async (command, payload = {}) => {
    const reply = await window.__TAURI_INTERNALS__.invoke(command, { request: { schemaVersion: 1, requestId: crypto.randomUUID(), payload } });
    if (reply.error) throw Error(command + ':' + JSON.stringify(reply.error));
    return reply.data;
  };
  const visible = e => !!e && !!e.getClientRects().length;
  const modal = () => [...document.querySelectorAll('dialog[open],[role=dialog]')].find(visible);
  const button = (text, root = document) => [...root.querySelectorAll('button')].find(e => visible(e) && e.textContent.trim() === text);
  const click = async (text, root = document) => { const b = await wait(text, () => { const b = button(text, root); return b && !b.disabled && b; }); b.scrollIntoView({block:'nearest'}); b.focus({preventScroll:true}); b.click(); await new Promise(r => setTimeout(r, 150)); };
  const panel = () => document.querySelector('[aria-label="首次设置与后台服务"]');
  let sequence = 0;
  const control = async (action, nonce = window.__ommRecoveryNonce) => {
    const id = 'recovery-' + (++sequence); window.__ommRecoveryControlResult = null;
    window.__ommRecoveryControl = { nonce, id, action };
    return wait(action, () => window.__ommRecoveryControlResult?.id === id && window.__ommRecoveryControlResult);
  };
  let titlebarInset;
  const snapshot = async request => {
    window.__ommFixtureSnapshotResult = null; window.__ommFixtureSnapshotRequest = request;
    const shot = await wait(request.name, () => window.__ommFixtureSnapshotResult);
    check('actual owned WK snapshot ' + request.name, shot.ok && shot.bytes > 0);
    return shot;
  };
  const reachable = async element => {
    element.scrollIntoView({block:'nearest', inline:'nearest'});
    await new Promise(r => setTimeout(r, 40));
    const rect = element.getBoundingClientRect();
    const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
    return rect.width > 0 && rect.height > 0 && rect.left >= 0 && rect.right <= innerWidth + 1 && rect.top >= 0 && rect.bottom <= innerHeight + 1 && !!hit && (hit === element || element.contains(hit));
  };
  const capture = async (stage) => {
    for (const [width, height, theme, collapsed] of [[800, 600, 'light', false], [1280, 820, 'dark', true]]) {
      const select = document.querySelector('select[aria-label="外观主题"]');
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(select, theme);
      select.dispatchEvent(new Event('change', {bubbles:true}));
      const toggle = document.querySelector(`[aria-label="${collapsed ? '收起侧栏' : '展开侧栏'}"]`);
      if (toggle) toggle.click();
      const name = `${stage}-${width}-${height}-${theme}-${collapsed ? 'collapsed' : 'expanded'}`;
      if (titlebarInset === undefined) {
        await snapshot({name:'viewport-calibration', width, height});
        titlebarInset = height - innerHeight;
        check('measured owned WK titlebar inset', titlebarInset >= 0 && titlebarInset <= 100);
      }
      await snapshot({name:name + '-size', width, height:height + titlebarInset});
      check('exact actual content dimensions ' + name, innerWidth === width && innerHeight === height);
      const doc = document.documentElement;
      const active = modal();
      let errorReachable = null, enabledControls = 0;
      if (active) {
        const focused = document.activeElement;
        window.__ommFixtureFocusEvidence = {name, documentFocused:document.hasFocus(), tag:focused?.tagName, aria:focused?.getAttribute('aria-label'), text:focused?.textContent?.slice(0,160), inDialog:active.contains(focused), disabled:focused?.disabled};
        check('actual dialog focus retained ' + name, active.contains(document.activeElement));
        check('actual dialog focused target remains usable ' + name, !document.activeElement.matches(':disabled'));
        const error = active.querySelector('[role=alert]');
        if (error) {
          errorReachable = await reachable(error);
          check('actual error reachable ' + name, errorReachable);
        }
        for (const control of [...active.querySelectorAll('button')].filter(b => !b.disabled && visible(b))) {
          check('enabled dialog control reachable ' + name + ':' + control.textContent.trim(), await reachable(control));
          enabledControls++;
        }
      }
      // Capture after resize and scroll/hit checks; never re-resize this settled content.
      const shot = await snapshot({name}); snapshots.push(shot);
      const over = Math.max(0, doc.scrollWidth - innerWidth);
      layouts.push({name, viewport:[innerWidth,innerHeight], windowSize:[width,height + titlebarInset], titlebarInset, scrollWidth:doc.scrollWidth, horizontalOverflow:over, dialog:!!active, errorReachable, enabledControls, theme:doc.dataset.mantineColorScheme, bodyBackground:getComputedStyle(document.body).backgroundColor, guidance:active?.querySelector('textarea')?.value ?? null});
      check('official Mantine theme active ' + name, doc.dataset.mantineColorScheme === theme);
      check('no page horizontal overflow ' + name, over <= 1);
    }
  };
  try {
    check('actual production origin', location.origin === 'tauri://localhost');
    check('bad fixture nonce denied', (await control('unloaded', crypto.randomUUID())).ok === false);
    const initial = await raw('config_get');
    check('missing recording fields remain null through native bridge', initial.autoRecord === null && initial.recordingConsent === null);
    await click('打开服务设置'); await wait('actual settings page', panel);
    await capture('unknown-preferences');
    await click('安全切换服务版本', panel());
    await wait('loaded job blocked real modal', () => modal()?.textContent.includes('旧自启服务仍被系统加载'));
    check('loaded old registration cannot be adopted', button('确认安全切换', modal())?.disabled);
    const loaded = await raw('service_plan', {action:'switch'});
    check('native exact unloaded reason', loaded.reasons.includes('external_unload_required') && !loaded.canApply && loaded.guidance.length > 0);
    await capture('external-unload-required');
    await click('暂不操作', modal());
    check('owned control marks synthetic job unloaded', (await control('unloaded')).ok);
    check('owned config fixture restored', (await control('restore-config')).ok);
    await click('安全切换服务版本', panel());
    await wait('actual adopt dialog', () => modal()?.textContent.includes('采用资料库并更新停机登记（不启动）'));
    await capture('adopt-confirmation');
    await click('确认采用（不启动）', modal());
    await wait('adoption dialog settled', () => !modal());
    await wait('actual adoption toast stopped', () => document.body.innerText.includes('采用完成，服务仍已停止'));
    check('adoption remains stopped in native core', (await raw('service_plan', {action:'start'})).state === 'stopped');
    operations.push('adopted-stopped'); await capture('adopted-stopped');
    await click('启动后台服务', panel());
    await wait('actual start modal', () => modal() && button('确认启动', modal()));
    check('private generation change admitted', (await control('change-generation')).ok);
    await click('确认启动', modal());
    await wait('real start failure displayed in same modal', () => modal()?.querySelector('[role=alert]'));
    check('failed start consumes plan', button('确认启动', modal()).disabled && modal().textContent.includes('本次计划已使用或失效'));
    operations.push('start-stale-refused'); await capture('start-error-consumed');
    await click('重新检查计划', modal());
    await wait('fresh start enabled', () => button('确认启动', modal()) && !button('确认启动', modal()).disabled);
    await capture('start-rechecked');
    await click('确认启动', modal()); await wait('start dialog settled', () => !modal());
    await wait('actual owned daemon running', async () => (await raw('service_plan', {action:'start'})).state === 'running');
    operations.push('start-running'); await capture('running');
    await click('停止后台服务', panel());
    await wait('actual stop confirmation', () => modal() && button('确认停止', modal()));
    await click('确认停止', modal()); await wait('stop completed', () => !modal());
    check('stopped owned daemon through real UI', (await raw('service_plan', {action:'start'})).state === 'stopped');
    operations.push('stop-stopped');
    check('no hardware playback called', playCalls === 0);
    check('production CSP admits static Mantine without violations', cspViolations.length === 0);
    window.__ommFixtureResult = {ok:true, checks, snapshots, layouts, operations, playCalls, cspViolations, boundary:'Actual Tauri/WK/React/native lifecycle; private launchctl job state and generation-control fixture only. Start failure is native stale-generation rejection, not simulated daemon crash.'};
  } catch (error) {
    window.__ommFixtureResult = {ok:false, checks, snapshots, layouts, operations, playCalls, focusEvidence:window.__ommFixtureFocusEvidence, error:String(error), body:document.body.innerText.slice(0,20000)};
  }
})();
