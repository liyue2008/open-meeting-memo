(async () => {
  const checks = [];
  let playCalls = 0;
  HTMLMediaElement.prototype.play = function () { playCalls++; throw Error('forbidden play'); };
  const check = (value, name) => {
    if (!value) throw Error(name);
    checks.push(name);
    window.__ommLifecycleDebug = name;
  };
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const call = async (command, payload = {}) => {
    window.__ommLifecycleDebug = `calling ${command}:${payload.action || ''}`;
    const value = await window.__TAURI_INTERNALS__.invoke(command, {
      request: { schemaVersion: 1, requestId: crypto.randomUUID(), payload },
    });
    if (value.error) throw Error(`${command}:${JSON.stringify(value.error)}`);
    return value.data;
  };
  const stage = async name => {
    window.__ommLifecycleDebug = `native ${name}`;
    window.__ommLifecycleCaseResult = null;
    window.__ommLifecycleCase = name;
    const deadline = performance.now() + 60_000;
    while (performance.now() < deadline) {
      await pause(150);
      const value = window.__ommLifecycleCaseResult;
      if (value?.case === name) { check(value.ok, `native ${name}`); return value; }
    }
    throw Error(`case timeout ${name}`);
  };
  let controlId = 0;
  const control = async (action, extra = {}, expected = true) => {
    const request = { id: `held-${++controlId}`, action, ...extra };
    const { id } = request;
    window.__ommLifecycleControlResult = null;
    window.__ommLifecycleControl = request;
    const deadline = performance.now() + 10_000;
    while (performance.now() < deadline) {
      await pause(100);
      const value = window.__ommLifecycleControlResult;
      if (value?.id === id) {
        check(value.ok === expected, `${action}:${id}:${JSON.stringify(value)}`);
        return value;
      }
    }
    throw Error(`control timeout ${action}`);
  };
  const rejectedApply = async (token, consumed = false) => {
    const value = await window.__TAURI_INTERNALS__.invoke('service_apply', {
      request: { schemaVersion: 1, requestId: crypto.randomUUID(), payload: { planToken: token } },
    });
    check(value.error?.code === 'conflict', 'real Core rejected old plan');
    if (consumed) check(!value.data, 'retired token remains rejected');
  };
  const ready = async action => {
    const deadline = performance.now() + 60_000;
    while (performance.now() < deadline) {
      const plan = await call('service_plan', { action });
      if (plan.canApply) return plan;
      await pause(200);
    }
    throw Error(`plan not ready ${action}`);
  };
  const apply = async action => {
    const plan = await ready(action);
    const value = await call('service_apply', { planToken: plan.planToken });
    check(value.state === (action === 'stop' ? 'stopped' : 'running'), `real CLI ${action}`);
  };
  try {
    const initial = await call('service_plan', { action: 'start' });
    check(initial.state === 'uninitialized' && initial.canInitialize, 'genuinely uninitialized');
    const initialized = await call('profile_initialize');
    check(initialized.initialized && !initialized.autoRecord && !initialized.values.notificationsEnabled,
      'explicit initialized safe defaults');
    await stage('configure');
    await control('hold-busy', {}, false); // Invalid before a completed running case.
    const duplicate = await control('restore', { id: 'held-1' }, false);
    check(duplicate.error.includes('duplicate'), 'duplicate control id refused');
    await control('hold-unknown', { path: '/not-accepted' }, false);
    const shown = await ready('start');
    const stale = await control('stale-plan');
    check(stale.held === 'stale' && stale.details.observedPlan.compatibility.serviceGeneration !== shown.compatibility.serviceGeneration, 'real profile generation held stale');
    check(Date.now() < shown.expiresAtMs, 'old plan not expired before stale apply');
    await rejectedApply(shown.planToken);
    await rejectedApply(shown.planToken, true);
    await control('restore');
    check(Date.now() < shown.expiresAtMs, 'old plan still unexpired after generation restore');
    await rejectedApply(shown.planToken, true);
    check((await ready('start')).compatibility.serviceGeneration === shown.compatibility.serviceGeneration, 'original generation restored');
    const unknown = await control('hold-unknown');
    check(unknown.held === 'unknown' && unknown.details.listenerPid > 0, 'independent owned listener held');
    for (let i = 0; i < 2; i++) {
      const observed = await call('service_plan', { action: 'start' });
      check(observed.state === 'unknown' && !observed.canApply && observed.reasons.includes('unowned_service'), 'real plan observes stable unknown');
    }
    const invalidHeld = await control('hold-busy', {}, false);
    check(invalidHeld.cleanup?.waited && invalidHeld.cleanup.socketRemovedAndLocksReleased, 'invalid request restores/waits unknown child');
    await control('hold-unknown');
    const restored = await control('restore');
    check(restored.details.waited && restored.details.socketRemovedAndLocksReleased, 'explicit restore waits unknown child');
    const before = await call('config_get');
    await apply('start');
    await ready('stop');
    await stage('running');
    const stoppable = await ready('stop');
    await control('hold-busy');
    for (let i = 0; i < 2; i++) {
      const busy = await call('service_plan', { action: 'stop' });
      check(busy.state === 'busy' && !busy.canApply && busy.reasons.includes('service_busy'), 'real Store holds busy/unsealed across observations');
    }
    await rejectedApply(stoppable.planToken);
    const cleared = await control('restore');
    check(cleared.details.recordingCleared, 'held unsealed sentinel safely cleared');
    await ready('stop');
    await apply('switch');
    await ready('stop');
    await stage('switched');
    await apply('stop');
    await stage('offline');
    await control('hold-unknown');
    const finish = await control('finish');
    check(finish.details.waited && finish.details.socketRemovedAndLocksReleased, 'finish restores and waits listener');
    await control('hold-unknown', {}, false);
    check((await ready('start')).canApply, 'normal next stage available after finish');
    const after = await call('config_get');
    check(before.revision === after.revision && JSON.stringify(before.values) === JSON.stringify(after.values),
      'configuration retained across start/switch/stop');
    window.__ommFixtureResult = { ok: true, checks, playCalls };
  } catch (error) {
    window.__ommFixtureResult = { ok: false, checks, playCalls, error: String(error) };
  }
})();
