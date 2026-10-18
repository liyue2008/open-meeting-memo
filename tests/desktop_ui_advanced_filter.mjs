import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import vm from 'node:vm';
import test from 'node:test';

const require = createRequire(new URL('../apps/desktop/package.json', import.meta.url));
const { JSDOM } = await import(pathToFileURL(require.resolve('jsdom')).href);
const source = readFileSync(new URL('../scripts/fixtures/desktop-ui-probe.js', import.meta.url), 'utf8');
const queries = source.slice(source.indexOf('  const visible ='), source.indexOf('  const change ='));
const observationStart = source.indexOf('  let advancedFilterObservation =');
const advanced = source.slice(observationStart < 0 ? source.indexOf('  const openAdvanced =') : observationStart, source.indexOf('  const modalCheckpoint ='));
const failureAssignment = source.slice(source.lastIndexOf('    window.__ommFixtureResult ='), source.lastIndexOf('\n  }\n})();'));

function fixture({ expanded = false, activate, onTick, missingControl = false, rect, style, throwGeometry = false, frames = 'missing' } = {}) {
  const dom = new JSDOM(`<button id="advanced-control" aria-controls="advanced-panel" aria-expanded="${expanded}">高级筛选与数据说明</button><section id="advanced-panel" role="region" aria-labelledby="advanced-control"><label>排序<select><option>开始时间 ↓</option></select></label></section>`);
  const { document, HTMLElement } = dom.window;
  // Pure DOM tests supply only visible/hidden geometry, not WK layout evidence.
  HTMLElement.prototype.getClientRects = function () {
    for (let node = this; node; node = node.parentElement) {
      if (node.hidden || node.style.display === 'none') return [];
    }
    return this.isConnected ? [{ width: 100, height: 20 }] : [];
  };
  HTMLElement.prototype.scrollIntoView = function () { state.operations.push('scroll'); };
  HTMLElement.prototype.getBoundingClientRect = function () {
    if (throwGeometry) throw Error('diagnostic geometry unavailable');
    return rect ?? { x: 0, y: 0, width: 100, height: 20 };
  };
  Object.defineProperty(document.body, 'innerText', { get() { return this.textContent; } });
  let clock = 0;
  const state = {
    document,
    control: document.getElementById('advanced-control'),
    panel: document.getElementById('advanced-panel'),
    sort: document.querySelector('select'),
    clicks: 0,
    operations: [],
    replaceControl(nextExpanded) {
      const next = this.control.cloneNode(true);
      next.setAttribute('aria-expanded', String(nextExpanded));
      this.control.replaceWith(next);
      return next;
    },
  };
  const pendingFrames = new Map(), registeredFrames = new Map();
  let frameId = 0;
  Object.assign(state, {
    pendingFrames, registeredFrames, frameRequests: 0, frameCancels: [],
    pulse() {
      const next = pendingFrames.entries().next().value;
      if (!next) return false;
      pendingFrames.delete(next[0]);
      next[1]();
      return true;
    },
  });
  if (frames !== 'missing') {
    dom.window.requestAnimationFrame = callback => {
      state.frameRequests++;
      if (frames === 'request-throws' || (frames === 'second-request-throws' && state.frameRequests === 2)) throw Error('private registration detail');
      const id = ++frameId;
      pendingFrames.set(id, callback);
      registeredFrames.set(id, callback);
      return id;
    };
    if (frames !== 'no-cancel') dom.window.cancelAnimationFrame = id => {
      state.frameCancels.push(id);
      if (frames === 'cancel-throws') throw Error('private cancellation detail');
      pendingFrames.delete(id);
    };
  }
  state.control.addEventListener('click', () => {
    state.clicks++;
    state.operations.push('click');
    if (activate) activate(state);
    else state.control.setAttribute('aria-expanded', 'true');
  });
  if (missingControl) state.control.remove();
  const context = vm.createContext({
    document, window: dom.window,
    checks: [], playCalls: 0, views: [], focusEvidence: [], mediaEvidence: [], reads: [],
    getComputedStyle: node => style ?? dom.window.getComputedStyle(node),
    Date: { now: () => clock },
    tick: async () => { state.operations.push('tick'); clock += 10001; onTick?.(state); },
  });
  vm.runInContext(queries + advanced + `
    globalThis.open = openAdvanced;
    globalThis.observation = () => JSON.parse(JSON.stringify(advancedFilterObservation));
    globalThis.failureResult = error => { ${failureAssignment} return window.__ommFixtureResult; };
  `, context);
  return { state, open: context.open, observation: () => JSON.parse(JSON.stringify(context.observation())), failureResult: context.failureResult, close: () => dom.window.close() };
}

async function run(options, expectation) {
  const f = fixture(options);
  try { await expectation(f); } finally { f.close(); }
}

const rejected = f => assert.rejects(f.open(), /Timeout: actual advanced Accordion expanded/);

test('advanced filter activates one real control and accepts an already open panel without toggling', async () => {
  await run({}, async f => { await f.open(); assert.equal(f.state.clicks, 1); });
  await run({ expanded: true }, async f => { await f.open(); assert.equal(f.state.clicks, 0); });
});

test('advanced filter follows the live expanded replacement instead of its detached initial control', async () => {
  await run({ activate: s => s.replaceControl(true) }, async f => {
    await f.open();
    assert.equal(f.state.control.isConnected, false);
    assert.equal(f.state.document.getElementById('advanced-control').getAttribute('aria-expanded'), 'true');
    assert.equal(f.state.clicks, 1);
  });
});

test('advanced filter re-queries replacement during the original wait without another click', async () => {
  await run({ activate: () => {}, onTick: s => {
    if (s.control.isConnected) s.replaceControl(true);
  } }, async f => { await f.open(); assert.equal(f.state.clicks, 1); });
});

test('advanced filter never accepts a detached expanded control while the live control is collapsed', async () => {
  await run({ activate: s => {
    s.control.setAttribute('aria-expanded', 'true');
    s.replaceControl(false);
  } }, rejected);
});

test('advanced filter still rejects an ineffective click and a missing sort field', async () => {
  await run({ activate: () => {} }, rejected);
  await run({ activate: s => {
    s.control.setAttribute('aria-expanded', 'true');
    s.sort.remove();
  } }, rejected);
});

test('advanced filter requires a visible enabled sort field', async () => {
  for (const unavailable of [s => { s.sort.hidden = true; }, s => { s.sort.disabled = true; }]) {
    await run({ activate: s => {
      s.control.setAttribute('aria-expanded', 'true');
      unavailable(s);
    } }, rejected);
  }
});

test('advanced filter cannot borrow a sort field from another panel', async () => {
  await run({ activate: s => {
    s.control.setAttribute('aria-expanded', 'true');
    s.sort.parentElement.remove();
    s.document.body.insertAdjacentHTML('beforeend', '<section><label>排序<select></select></label></section>');
  } }, rejected);
});

test('advanced filter rejects a panel that does not belong to the live control', async () => {
  await run({ activate: s => {
    s.control.setAttribute('aria-expanded', 'true');
    s.panel.setAttribute('aria-labelledby', 'different-control');
  } }, rejected);
});

const lastSnapshot = f => f.observation().snapshots.at(-1);

test('advanced frames distinguish zero, one and two real helper callbacks without recurring', async () => {
  for (const count of [0, 1, 2]) {
    let pulsed = false;
    await run({ frames: 'available', activate: () => {}, onTick: s => {
      if (!pulsed) { for (let i = 0; i < count; i++) assert.equal(s.pulse(), true); pulsed = true; }
    } }, async f => {
      await timeout(f);
      assert.deepEqual(f.observation().snapshots.map(s => s.frames), [
        { callbacks: 0, status: 'not-called' },
        { callbacks: 0, status: 'scheduled' },
        { callbacks: count, status: count === 2 ? 'complete' : 'scheduled' },
      ]);
      assert.equal(f.observation().frames.cleanup, count === 2 ? 'not-needed' : 'cancelled');
      assert.equal(f.state.frameRequests, count === 0 ? 1 : 2);
      assert.equal(f.state.pendingFrames.size, 0);
      assert.equal(f.state.pulse(), false);
      assert.deepEqual(f.state.operations, ['scroll', 'click', 'tick', 'tick']);
    });
  }
});

test('advanced frames distinguish no invocation, unavailable APIs and first or second registration failures', async () => {
  await run({ frames: 'available', missingControl: true }, async f => {
    await timeout(f);
    assert.deepEqual(f.observation().frames, { callbacks: 0, status: 'not-called', cleanup: 'not-needed' });
    assert.equal(f.state.frameRequests, 0);
  });
  for (const frames of ['missing', 'no-cancel', 'request-throws', 'second-request-throws']) {
    await run({ frames, activate: () => {}, onTick: s => { s.pulse(); } }, async f => {
      await timeout(f);
      const observed = f.observation().frames;
      assert.equal(observed.status, frames.endsWith('throws') ? 'schedule-failed' : 'unavailable');
      assert.equal(observed.callbacks, frames === 'second-request-throws' ? 1 : 0);
      assert.equal(observed.cleanup, 'not-needed');
      assert.equal(JSON.stringify(f.observation()).includes('private'), false);
      assert.equal(f.state.pendingFrames.size, 0);
      if (frames === 'no-cancel') assert.equal(f.state.frameRequests, 0);
    });
  }
});

test('advanced frames clean only owned work after success and reset on the next helper invocation', async () => {
  await run({ frames: 'available', expanded: true }, async f => {
    let unrelated = 0;
    const other = f.state.document.defaultView.requestAnimationFrame(() => { unrelated++; });
    await f.open();
    const first = f.observation();
    assert.equal(first.frames.cleanup, 'cancelled');
    assert.deepEqual(f.state.frameCancels, [2]);
    assert.equal(f.state.pendingFrames.has(other), true);
    f.state.registeredFrames.get(2)(); // A cancelled callback cannot mutate its stopped invocation.
    assert.deepEqual(f.observation(), first);
    assert.equal(f.state.pulse(), true);
    assert.equal(unrelated, 1);
    f.state.control.setAttribute('aria-expanded', 'false');
    await f.open();
    assert.equal(f.state.clicks, 1);
    assert.equal(f.observation().snapshots.length, 2);
    assert.deepEqual(f.observation().snapshots[0].frames, { callbacks: 0, status: 'not-called' });
    assert.deepEqual(f.observation().frames, { callbacks: 0, status: 'scheduled', cleanup: 'cancelled' });
    assert.deepEqual(f.state.frameCancels, [2, 3]);
    const second = f.observation();
    f.state.registeredFrames.get(2)();
    assert.deepEqual(f.observation(), second);
    assert.equal(f.state.pendingFrames.size, 0);
  });
});

test('advanced frame cleanup and inline collection exceptions never replace the original failure', async () => {
  const original = Error('original frame test failure');
  await run({ frames: 'cancel-throws', activate: () => {}, throwGeometry: true, onTick: () => { throw original; } }, async f => {
    await assert.rejects(f.open(), error => error === original);
    assert.equal(f.failureResult(original).error, String(original));
    assert.equal(lastSnapshot(f).read, 'unavailable');
    assert.equal(lastSnapshot(f).panel.inlineDisplay, 'unavailable');
    assert.equal(f.observation().frames.cleanup, 'failed');
    const before = f.observation();
    assert.equal(f.state.pulse(), true);
    assert.deepEqual(f.observation(), before);
    assert.equal(f.state.frameRequests, 1);
    assert.deepEqual(f.state.operations, ['scroll', 'click', 'tick']);
  });
  for (const [value, expected] of [['', 'empty'], ['none', 'none'], ['block', 'block'], ['flex', 'other'], ['https://private.invalid/path', 'other']]) {
    await run({ activate: () => {} }, async f => {
      Object.defineProperty(f.state.panel.style, 'display', { get: () => value });
      await timeout(f);
      assert.equal(lastSnapshot(f).panel.inlineDisplay, expected);
      assert.equal(JSON.stringify(f.observation()).includes('private.invalid'), false);
    });
  }
  const inlineWaitError = Error('original wait before diagnostic inline failure');
  await run({ activate: () => {}, onTick: s => {
    // Install after the original predicate saved its references, so only the final diagnostic reads it.
    Object.defineProperty(s.panel.style, 'display', { get: () => { throw Error('private style detail'); } });
    throw inlineWaitError;
  } }, async f => {
    await assert.rejects(f.open(), error => error === inlineWaitError);
    assert.equal(f.failureResult(inlineWaitError).error, String(inlineWaitError));
    assert.equal(lastSnapshot(f).read, 'unavailable');
    assert.equal(lastSnapshot(f).panel.inlineDisplay, 'unavailable');
  });
  await run({ frames: 'cancel-throws', expanded: true }, async f => {
    await f.open();
    assert.equal(f.observation().frames.cleanup, 'failed');
    const before = f.observation();
    assert.equal(f.state.pulse(), true);
    assert.deepEqual(f.observation(), before);
    assert.equal(f.state.frameRequests, 1);
    assert.equal(f.state.clicks, 0);
  });
});

async function timeout(f) {
  let original;
  try { await f.open(); } catch (error) { original = error; }
  assert.match(String(original), /Timeout: actual advanced Accordion/);
  const result = f.failureResult(original);
  assert.equal(result.ok, false);
  assert.equal(result.error, String(original));
  assert.deepEqual(JSON.parse(JSON.stringify(result.advancedFilterObservation)), f.observation());
  return result;
}

test('advanced observation distinguishes no click, one ineffective click, already open, and a closed replacement', async () => {
  await run({ missingControl: true }, async f => {
    await timeout(f);
    assert.equal(lastSnapshot(f).clickCalls, 0);
    assert.equal(lastSnapshot(f).current.exists, false);
    assert.deepEqual(f.state.operations, ['tick', 'tick']);
  });
  await run({ activate: () => {} }, async f => {
    await timeout(f);
    assert.deepEqual(f.observation().snapshots.map(s => s.phase), ['before-activation', 'after-activation', 'last-query']);
    assert.equal(lastSnapshot(f).clickCalls, 1);
    assert.equal(lastSnapshot(f).current.expanded, 'false');
    assert.deepEqual(f.state.operations, ['scroll', 'click', 'tick', 'tick']);
  });
  await run({ expanded: true }, async f => {
    await f.open();
    assert.equal(f.observation().snapshots.length, 2);
    assert.equal(lastSnapshot(f).clickCalls, 0);
    assert.deepEqual(f.state.operations, []);
  });
  await run({ activate: s => s.replaceControl(false) }, async f => {
    await timeout(f);
    const snapshot = lastSnapshot(f);
    assert.equal(snapshot.sameControl, false);
    assert.equal(snapshot.original.connected, false);
    assert.equal(snapshot.current.connected, true);
    assert.equal(snapshot.current.expanded, 'false');
    assert.equal(snapshot.clickCalls, 1);
  });
});

test('advanced observation identifies panel and field failures without weakening the original predicate', async () => {
  const cases = [
    [s => s.panel.remove(), s => assert.equal(s.panel.exists, false)],
    [s => s.control.removeAttribute('aria-controls'), s => assert.equal(s.controls, 'missing')],
    [s => s.panel.setAttribute('aria-labelledby', 'other-control'), s => assert.equal(s.labelledBy, 'mismatch')],
    [s => s.panel.removeAttribute('aria-labelledby'), s => assert.equal(s.labelledBy, 'missing')],
    [s => s.panel.setAttribute('role', 'other-role'), s => assert.equal(s.region, 'invalid')],
    [s => s.sort.remove(), s => assert.equal(s.sort.exists, false)],
    [s => { s.sort.hidden = true; }, s => assert.equal(s.sort.visible, false)],
    [s => { s.sort.disabled = true; }, s => assert.equal(s.sort.disabled, true)],
  ];
  for (const [invalidate, inspect] of cases) {
    await run({ activate: s => { s.control.setAttribute('aria-expanded', 'true'); invalidate(s); } }, async f => {
      await timeout(f);
      inspect(lastSnapshot(f));
      assert.equal(lastSnapshot(f).clickCalls, 1);
      assert.deepEqual(f.state.operations, ['scroll', 'click', 'tick', 'tick']);
    });
  }
});

test('advanced observation is capped and contains only fixed enums, booleans and finite bounded numbers', async () => {
  const secret = 'https://private.invalid/personal/meeting-content';
  await run({
    activate: () => {},
    rect: { x: Infinity, y: NaN, width: 1e50, height: -1e50 },
    style: { display: secret, visibility: secret },
  }, async f => {
    f.state.control.setAttribute('aria-expanded', secret);
    f.state.control.id = secret + '-control';
    f.state.panel.id = secret + '-panel';
    f.state.control.setAttribute('aria-controls', f.state.panel.id);
    f.state.panel.setAttribute('aria-labelledby', f.state.control.id);
    f.state.document.body.append(secret);
    await timeout(f);
    await timeout(f);
    const observation = f.observation();
    assert.deepEqual(Object.keys(observation).sort(), ['frames', 'schemaVersion', 'snapshots']);
    assert.equal(observation.schemaVersion, 2);
    assert.deepEqual(Object.keys(observation.frames).sort(), ['callbacks', 'cleanup', 'status']);
    assert.equal(observation.snapshots.length, 3);
    for (const snapshot of observation.snapshots) {
      assert.deepEqual(Object.keys(snapshot).sort(), ['clickCalls', 'controls', 'current', 'frames', 'labelledBy', 'original', 'panel', 'phase', 'read', 'region', 'sameControl', 'sort']);
      for (const control of [snapshot.original, snapshot.current]) assert.deepEqual(Object.keys(control).sort(), ['connected', 'disabled', 'exists', 'expanded', 'visible']);
      assert.deepEqual(Object.keys(snapshot.panel).sort(), ['connected', 'display', 'exists', 'inlineDisplay', 'rect', 'visibility', 'visible']);
      assert.deepEqual(Object.keys(snapshot.frames).sort(), ['callbacks', 'status']);
      assert.deepEqual(Object.keys(snapshot.sort).sort(), ['connected', 'disabled', 'display', 'exists', 'inPanel', 'rect', 'visibility', 'visible']);
      for (const node of [snapshot.panel, snapshot.sort]) assert.deepEqual(Object.keys(node.rect).sort(), ['height', 'width', 'x', 'y']);
    }
    assert.equal(f.state.clicks, 2); // One original click per invocation, not per diagnostic.
    assert.equal(JSON.stringify(observation).includes(secret), false);
    const allowed = new Set(['before-activation', 'after-activation', 'last-query', 'ok', 'unavailable', 'true', 'false', 'missing', 'other', 'match', 'mismatch', 'valid', 'invalid', 'none', 'block', 'inline', 'inline-block', 'flex', 'inline-flex', 'grid', 'inline-grid', 'contents', 'table', 'list-item', 'visible', 'hidden', 'collapse', 'empty', 'not-called', 'scheduled', 'complete', 'schedule-failed', 'not-needed', 'cancelled', 'failed']);
    const inspect = value => {
      if (typeof value === 'string') assert.ok(allowed.has(value), value);
      else if (typeof value === 'number') assert.ok(Number.isFinite(value) && Math.abs(value) <= 16384);
      else if (value && typeof value === 'object') Object.values(value).forEach(inspect);
      else assert.equal(typeof value, 'boolean');
    };
    inspect(observation);
    assert.deepEqual(lastSnapshot(f).panel.rect, { x: 0, y: 0, width: 16384, height: 0 });
    assert.equal(lastSnapshot(f).panel.display, 'other');
    assert.equal(lastSnapshot(f).panel.visibility, 'other');
  });
});

test('advanced diagnostic exceptions preserve the original Timeout, thrown error, and failure envelope', async () => {
  await run({ activate: () => {}, throwGeometry: true }, async f => {
    assert.equal(f.failureResult(Error('before advanced operation')).advancedFilterObservation, null);
    const result = await timeout(f);
    assert.equal(result.error, 'Error: Timeout: actual advanced Accordion expanded');
    assert.equal(result.playCalls, 0);
    assert.equal(result.independentNativeReads.length, 0);
    assert.ok(f.observation().snapshots.every(s => s.read === 'unavailable'));
    assert.deepEqual(f.state.operations, ['scroll', 'click', 'tick', 'tick']);
  });
  const original = Error('original wait failure');
  await run({ activate: () => {}, throwGeometry: true, onTick: () => { throw original; } }, async f => {
    await assert.rejects(f.open(), error => error === original);
    assert.equal(f.failureResult(original).error, String(original));
    assert.equal(lastSnapshot(f).read, 'unavailable');
    assert.deepEqual(f.state.operations, ['scroll', 'click', 'tick']);
  });
});
