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
    const before = await call('config_get');
    await apply('start');
    await ready('stop');
    await stage('running');
    await apply('switch');
    await ready('stop');
    await stage('switched');
    await apply('stop');
    await stage('offline');
    const after = await call('config_get');
    check(before.revision === after.revision && JSON.stringify(before.values) === JSON.stringify(after.values),
      'configuration retained across start/switch/stop');
    window.__ommFixtureResult = { ok: true, checks, playCalls };
  } catch (error) {
    window.__ommFixtureResult = { ok: false, checks, playCalls, error: String(error) };
  }
})();
