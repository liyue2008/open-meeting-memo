import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = readFileSync(new URL('../scripts/fixtures/desktop-ui-probe.js', import.meta.url), 'utf8');
const helper = source.slice(source.indexOf('  const stageTraceStarted'), source.indexOf('  const invoke ='));
const makeTrace = enabled => {
  const window = enabled === undefined ? {} : {__ommFixtureStageTraceEnabled: enabled};
  let clock = 10;
  const context = vm.createContext({window, performance: {now: () => ++clock}});
  vm.runInContext(helper + '\nglobalThis.record = stageTrace;', context);
  return {window, record: context.record};
};

test('desktop UI trace is off by default and only stores bounded numeric fixed stages', () => {
  const off = makeTrace(); off.record(14, 0); assert.equal(off.window.__ommFixtureStageTrace, undefined);
  const on = makeTrace(true); on.record(14, 0); on.record(14, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(on.window.__ommFixtureStageTrace)), [[14,0,1,1],[14,1,2,2]]);
  for (const input of [[27,0],[-1,0],['private',0],[0,'secret'],[0,3],[0,0.5]]) on.record(...input);
  assert.equal(on.window.__ommFixtureStageTrace.length, 2);
  for (let i=0; i<200; i++) on.record(26, 0);
  assert.equal(on.window.__ommFixtureStageTrace.length, 128);
  assert(on.window.__ommFixtureStageTrace.every(r => r.length === 4 && r.every(Number.isInteger)));
});
