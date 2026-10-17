// Actual React lifecycle controls; production transport is never replaced.
(async () => {
  const checks = [], stages = [], controls = [], views = [];
  let playCalls = 0;
  HTMLMediaElement.prototype.play = function () { playCalls++; throw Error('forbidden playback'); };
  const check = (name, value) => { if (!value) throw Error(name); checks.push(name); window.__ommLifecycleDebug = name; };
  const raw = async (command, payload = {}) => {
    const reply = await window.__TAURI_INTERNALS__.invoke(command, { request: { schemaVersion: 1, requestId: crypto.randomUUID(), payload } });
    if (reply.error) throw Error(command + ':' + JSON.stringify(reply.error));
    return reply.data;
  };
  const pause = () => new Promise(r => setTimeout(r, 100));
  const wait = async (name, predicate, budget = 45000) => {
    window.__ommLifecycleDebug = name;
    const end = Date.now() + budget;
    do { const value = await predicate(); if (value) return value; await pause(); } while (Date.now() < end);
    throw Error('Timeout: ' + name);
  };
  const visible = e => !!e && e.getClientRects().length > 0 && !e.closest('[hidden]');
  const buttons = (root = document) => [...root.querySelectorAll('button')].filter(visible);
  const find = (name, root = document) => buttons(root).find(e => e.textContent.trim() === name);
  const click = async (name, root = document) => {
    const b = await wait('enabled real button ' + name, () => { const x = find(name, root); return x && !x.disabled && x; });
    check('actual enabled button: ' + name, !b.disabled); b.click(); await pause();
  };
  const modal = () => [...document.querySelectorAll('dialog[open],[role=dialog]')].find(visible);
  const panel = () => document.querySelector('section[aria-label="首次设置与后台服务"]');
  const stage = async name => {
    window.__ommLifecycleCaseResult = null; window.__ommLifecycleCase = name;
    const value = await wait('native owned case ' + name, () => window.__ommLifecycleCaseResult?.case === name && window.__ommLifecycleCaseResult, 60000);
    check('owned native case: ' + name, value.ok); stages.push(value); return value;
  };
  let controlId = 0;
  const control = async action => {
    const id = `react-${++controlId}`;
    window.__ommLifecycleControlResult = null;
    window.__ommLifecycleControl = { id, action };
    const value = await wait('owned held control ' + action, () => window.__ommLifecycleControlResult?.id === id && window.__ommLifecycleControlResult, 15000);
    check('bounded owned control: ' + action, value.action === action && value.ok === true);
    controls.push(value); return value;
  };
  const rejectedPlan = async name => {
    await wait('real Core conflict visible: ' + name, () => modal()?.querySelector('[role=alert]')?.textContent.trim() === 'State changed or work is active; reload and retry（stale_revision）');
    check('rejected plan consumed in real UI: ' + name, find(name, modal()).disabled);
    views.push({ name: 'rejected-' + name, body: modal().innerText });
  };
  const plan = async (buttonName, confirmName) => {
    await click(buttonName, panel());
    await wait('real service plan modal ' + confirmName, () => modal() && find(confirmName, modal()));
    check('real plan describes native revalidation', modal().textContent.includes('原生端会再次核对'));
  };
  const apply = async (buttonName, confirmName, state) => {
    await plan(buttonName, confirmName); await click(confirmName, modal());
    await wait('actual apply completed: ' + state, () => !modal());
    await wait('native service state: ' + state, async () => (await raw('service_plan', { action: 'start' })).state === state);
    await click('重新检查服务状态', panel());
    await wait('real UI state after apply', () => panel().querySelector('strong')?.textContent === (state === 'running' ? '正在运行' : '已停止'));
    check('actual React action completed: ' + buttonName, !modal());
  };
  try {
    check('trusted actual WK origin', location.origin === 'tauri://localhost');
    const initial = await raw('service_plan', { action: 'start' });
    check('fresh actual profile needs explicit initialize', initial.state === 'uninitialized' && initial.canInitialize);
    await click('打开首次设置');
    await wait('real first-use panel', () => panel() && find('初始化本机资料库', panel()));
    await click('启动后台服务', panel());
    await wait('uninitialized actual plan', () => modal() && find('确认启动', modal()));
    check('uninitialized UI refuses apply', find('确认启动', modal()).disabled && modal().textContent.includes('尚未初始化'));
    await click('暂不操作', modal());
    await click('初始化本机资料库', panel()); await click('暂不初始化', modal());
    check('initialization cancellation did not mutate profile', (await raw('service_plan', { action: 'start' })).state === 'uninitialized');
    await click('初始化本机资料库', panel()); await click('确认初始化', modal());
    await wait('actual initialization dialog settled', () => !modal());
    const initialized = await raw('config_get');
    check('actual UI initialize yields safe config', initialized.initialized && initialized.autoRecord === false && initialized.recordingConsent === false && initialized.values.notificationsEnabled === false);
    await stage('configure');
    const before = await raw('config_get');
    await plan('启动后台服务', '确认启动');
    const originalGeneration = (await raw('service_plan', { action: 'start' })).compatibility.serviceGeneration;
    const stale = await control('stale-plan');
    check('actual held generation changed', stale.held === 'stale' && stale.details.observedPlan.compatibility.serviceGeneration !== originalGeneration);
    await click('确认启动', modal()); await rejectedPlan('确认启动');
    check('stale plan did not start service', (await raw('service_plan', { action: 'start' })).state === 'stopped');
    const staleRestore = await control('restore');
    check('exact original owner restored', staleRestore.details.originalOwnerRestored && (await raw('service_plan', { action: 'start' })).compatibility.serviceGeneration === originalGeneration);
    check('restoring generation does not replay consumed UI plan', find('确认启动', modal()).disabled);
    await click('重新检查计划', modal());
    await wait('new plan after stale is actionable', () => !find('确认启动', modal()).disabled);
    check('fresh stale recheck still needs confirmation', (await raw('service_plan', { action: 'start' })).state === 'stopped');
    await click('暂不操作', modal());
    const unknown = await control('hold-unknown');
    check('real independent unknown listener held', unknown.held === 'unknown' && unknown.details.listenerPid > 0);
    await click('重新检查服务状态', panel());
    await wait('unknown state visible in actual React', () => panel().querySelector('strong')?.textContent === '状态未知');
    for (const [buttonName, confirmName] of [['启动后台服务','确认启动'], ['停止后台服务','确认停止'], ['安全切换服务版本','确认安全切换']]) {
      await plan(buttonName, confirmName);
      check('actual unknown ownership refuses: ' + buttonName, find(confirmName, modal()).disabled && modal().textContent.includes('当前服务归属不明'));
      views.push({ name: 'unknown-' + buttonName, body: modal().innerText });
      await click('暂不操作', modal());
    }
    const unknownRestore = await control('restore');
    check('unknown listener waited and locks released', unknownRestore.details.waited && unknownRestore.details.socketRemovedAndLocksReleased);
    await click('重新检查服务状态', panel());
    await wait('restored stopped state visible', () => panel().querySelector('strong')?.textContent === '已停止');
    await plan('启动后台服务', '确认启动');
    const expiresNoEarlierThanUi = (await raw('service_plan', { action: 'start' })).expiresAtMs;
    check('genuine expiring native plan exists', Number.isFinite(expiresNoEarlierThanUi) && expiresNoEarlierThanUi > Date.now());
    await wait('actual wall-clock expiry without changing OS time', () => Date.now() > expiresNoEarlierThanUi + 100, 75000);
    if (!find('确认启动', modal()).disabled) {
      await click('确认启动', modal());
      await wait('real expired-plan UI guard', () => modal().textContent.includes('本次计划不可执行或已过期'));
    }
    check('expired real confirm disabled without implicit apply', find('确认启动', modal()).disabled && (await raw('service_plan', { action: 'start' })).state === 'stopped');
    await click('重新检查计划', modal());
    await wait('fresh real recheck enables confirm', () => !find('确认启动', modal()).disabled);
    check('fresh plan still requires explicit confirmation', (await raw('service_plan', { action: 'start' })).state === 'stopped');
    await click('确认启动', modal()); await wait('start dialog settled', () => !modal());
    await wait('real started CLI responds', async () => (await raw('service_plan', { action: 'stop' })).canApply);
    const running = await stage('running');
    await plan('停止后台服务', '确认停止');
    const heldBusy = await control('hold-busy');
    check('actual Store recording/unsealed held', heldBusy.held === 'busy');
    await click('确认停止', modal()); await rejectedPlan('确认停止');
    await click('重新检查计划', modal());
    await wait('busy reason visible in actual plan', () => modal().textContent.includes('有活动录音、未封存工作或正在处理的任务'));
    check('busy real confirm disabled', find('确认停止', modal()).disabled);
    views.push({ name: 'busy-held', body: modal().innerText });
    await click('暂不操作', modal());
    await click('重新检查服务状态', panel());
    await wait('busy state visible in real React', () => panel().querySelector('strong')?.textContent === '正在录音或处理任务');
    const busyRestore = await control('restore');
    check('owned unsealed sentinel safely cleared', busyRestore.details.recordingCleared);
    await click('重新检查服务状态', panel());
    await wait('restored owned running visible', () => panel().querySelector('strong')?.textContent === '正在运行');
    await apply('安全切换服务版本', '确认安全切换', 'running');
    const switched = await stage('switched');
    check('switch owns a different actual CLI process', running.ownedPid !== switched.ownedPid);
    await apply('停止后台服务', '确认停止', 'stopped');
    await stage('offline');
    await control('hold-unknown');
    await plan('启动后台服务', '确认启动');
    check('offline held unknown refuses actual UI apply', find('确认启动', modal()).disabled && modal().textContent.includes('当前服务归属不明'));
    await click('暂不操作', modal());
    const finished = await control('finish');
    check('finish restores and waits listener plus capture peer', finished.details.waited && finished.details.socketRemovedAndLocksReleased && finished.details.capturePeerWaited.waited && finished.details.capturePeerWaited.exitCode === 0);
    check('finish never starts service implicitly', (await raw('service_plan', { action: 'start' })).state === 'stopped');
    const after = await raw('config_get');
    check('actual React start switch stop retains configuration', before.revision === after.revision && JSON.stringify(before.values) === JSON.stringify(after.values));
    views.push({ name: 'first-use-and-service-success', body: panel().innerText });
    window.__ommFixtureResult = { ok: true, checks, stages, controls, views, playCalls, actualReactLifecycle: true };
  } catch (error) {
    window.__ommFixtureResult = { ok: false, checks, stages, controls, views, playCalls, error: String(error), body: document.body.innerText.slice(0, 16000) };
  }
})();
