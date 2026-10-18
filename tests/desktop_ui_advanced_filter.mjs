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
const advanced = source.slice(source.indexOf('  const openAdvanced ='), source.indexOf('  const modalCheckpoint ='));

function fixture({ expanded = false, activate, onTick } = {}) {
  const dom = new JSDOM(`<button id="advanced-control" aria-controls="advanced-panel" aria-expanded="${expanded}">高级筛选与数据说明</button><section id="advanced-panel" role="region" aria-labelledby="advanced-control"><label>排序<select><option>开始时间 ↓</option></select></label></section>`);
  const { document, HTMLElement } = dom.window;
  // Pure DOM tests supply only visible/hidden geometry, not WK layout evidence.
  HTMLElement.prototype.getClientRects = function () {
    for (let node = this; node; node = node.parentElement) {
      if (node.hidden || node.style.display === 'none') return [];
    }
    return this.isConnected ? [{ width: 100, height: 20 }] : [];
  };
  HTMLElement.prototype.scrollIntoView = function () {};
  let clock = 0;
  const state = {
    document,
    control: document.getElementById('advanced-control'),
    panel: document.getElementById('advanced-panel'),
    sort: document.querySelector('select'),
    clicks: 0,
    replaceControl(nextExpanded) {
      const next = this.control.cloneNode(true);
      next.setAttribute('aria-expanded', String(nextExpanded));
      this.control.replaceWith(next);
      return next;
    },
  };
  state.control.addEventListener('click', () => {
    state.clicks++;
    if (activate) activate(state);
    else state.control.setAttribute('aria-expanded', 'true');
  });
  const context = vm.createContext({
    document,
    Date: { now: () => clock },
    tick: async () => { clock += 10001; onTick?.(state); },
  });
  vm.runInContext(queries + advanced + '\nglobalThis.open = openAdvanced;', context);
  return { state, open: context.open, close: () => dom.window.close() };
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
