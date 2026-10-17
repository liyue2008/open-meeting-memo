// Private native fixture only: rebuild this worktree's production React assets and use the real IPC bridge.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
const root = fileURLToPath(new URL('../../../', import.meta.url));
const evidence = fs.mkdtempSync('/private/tmp/omm-ui-production-wk-');
const nativeProbe = fs.readFileSync(
  path.join(root, 'apps/desktop/src-tauri/fixtures/bridge-probe.js'),
  'utf8',
);
const completion = 'window.__ommFixtureResult={ok:true,checks,playCalls};';
assert.equal(nativeProbe.split(completion).length, 2, 'native probe completion seam changed');
const extraChecks = `
 const nativeInfo = await call('desktop_info', sample('desktop_info'));
 const titles = [...document.querySelectorAll('.meeting-title')].map(node => node.textContent);
 check('production React renders real persistent meeting library', titles.some(title => /Private bridge fixture|Edited synthetic title/.test(title)));
 check('production React renders real desktop_info version', document.querySelector('.page-footer')?.textContent.includes('v' + nativeInfo.appVersion) && !document.querySelector('.page-footer')?.textContent.includes('正在读取版本'));
 check('production React library has no validation error', !document.querySelector('[aria-label="会议列表"] [role="alert"]'));
 check('production React lifecycle plan validated', !document.querySelector('[aria-label="本机首次设置与服务状态"]')?.textContent.includes('无法核实'));
 window.__ommFixtureResult={ok:true,checks,playCalls,titles,nativeVersion:nativeInfo.appVersion,alerts:[...document.querySelectorAll('[role="alert"]')].map(node=>node.textContent),body:document.body.innerText,html:document.documentElement.outerHTML.slice(0,18000)};
`;
const driver = path.join(evidence, 'production-ui-driver.js');
fs.writeFileSync(driver, nativeProbe.replace(completion, extraChecks), { mode: 0o600 });
const log = fs.openSync(path.join(evidence, 'runner.log'), 'w', 0o600);
console.log(`Production WKWebView evidence: ${evidence}`);
const result = spawnSync('bash', [path.join(root, 'packaging/desktop/test-wkwebview.sh')], {
  cwd: root,
  env: {
    ...process.env,
    OMM_WKWEBVIEW_EVIDENCE: path.join(evidence, 'fixture'),
    OMM_WKWEBVIEW_DRIVER: driver,
  },
  stdio: ['ignore', log, log],
});
fs.closeSync(log);
fs.writeFileSync(path.join(evidence, 'runner.exit'), `${result.status}\n`, { mode: 0o600 });
const hashes = {};
for (const relative of [
  'apps/desktop/scripts/generate-validators.mjs',
  'apps/desktop/src/bridge/validators.generated.js',
  'apps/desktop/src-tauri/fixtures/bridge-probe.js',
  ...fs
    .readdirSync(path.join(root, 'apps/desktop/dist/assets'))
    .map((name) => 'apps/desktop/dist/assets/' + name),
])
  hashes[relative] = createHash('sha256')
    .update(fs.readFileSync(path.join(root, relative)))
    .digest('hex');
fs.writeFileSync(
  path.join(evidence, 'source-and-assets.sha256.json'),
  JSON.stringify(hashes, null, 2) + '\n',
  { mode: 0o600 },
);
assert.ifError(result.error);
assert.equal(result.status, 0, `native fixture failed; inspect ${evidence}/runner.log`);
const report = JSON.parse(
  fs.readFileSync(path.join(evidence, 'fixture/bridge-result.json'), 'utf8'),
).result;
assert.equal(report.ok, true);
assert.equal(report.playCalls, 0);
for (const check of [
  'production React renders real persistent meeting library',
  'production React renders real desktop_info version',
  'production React library has no validation error',
  'production React lifecycle plan validated',
])
  assert.ok(report.checks.includes(check), check);
console.log(
  `Production WKWebView: ${report.checks.length} checks passed; version=${report.nativeVersion}; playCalls=0.`,
);
console.log(
  `Remaining DOM alerts (native close permission is a separate repair): ${JSON.stringify(report.alerts)}`,
);
