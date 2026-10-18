// Run explicitly: node tests/ui-layout-wkwebview/run.mjs /private/tmp/omm-ui-layout-XXXXXX
// Component evidence only: copied current React, real nonpersistent WKWebView, synthetic bridge.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';
import react from '@vitejs/plugin-react';
const here = path.dirname(fileURLToPath(import.meta.url));
const desktop = path.resolve(here, '../..');
const out = process.argv[2];
if (!out || !path.resolve(out).startsWith('/private/tmp/'))
  throw new Error('Explicit private /private/tmp output required');
fs.mkdirSync(out, { recursive: true, mode: 0o700 });
if (fs.readdirSync(out).length) throw new Error('Output directory must be empty');
const fixture = path.join(out, 'fixture');
fs.mkdirSync(fixture, { mode: 0o700 });
fs.cpSync(path.join(desktop, 'src'), path.join(fixture, 'src'), { recursive: true });
fs.mkdirSync(path.join(fixture, 'tests/ui-layout-wkwebview'), { recursive: true });
fs.copyFileSync(
  path.join(here, 'fixture.jsx'),
  path.join(fixture, 'tests/ui-layout-wkwebview/fixture.jsx'),
);
fs.symlinkSync(path.join(desktop, 'node_modules'), path.join(fixture, 'node_modules'));
const samples = {};
for (const name of fs
  .readdirSync(path.join(desktop, '../../contracts/desktop/v1/examples'))
  .sort()) {
  if (!name.endsWith('.json')) continue;
  const value = JSON.parse(
    fs.readFileSync(path.join(desktop, '../../contracts/desktop/v1/examples', name), 'utf8'),
  );
  for (const exchange of Array.isArray(value) ? value : [value])
    if ('data' in exchange.response) samples[exchange.command] ??= exchange.response.data;
}
fs.writeFileSync(
  path.join(fixture, 'index.html'),
  '<div id="root"></div><script type="module" src="/tests/ui-layout-wkwebview/fixture.jsx"></script>',
);
await build({
  root: fixture,
  configFile: false,
  plugins: [react()],
  base: './',
  build: { target: 'safari18.5', outDir: 'dist' },
});
const assets = fs.readdirSync(path.join(fixture, 'dist/assets'));
const css = assets
  .filter((n) => n.endsWith('.css'))
  .map((n) => fs.readFileSync(path.join(fixture, 'dist/assets', n), 'utf8'))
  .join('\n');
const js = assets
  .filter((n) => n.endsWith('.js'))
  .map((n) => fs.readFileSync(path.join(fixture, 'dist/assets', n), 'utf8'))
  .join('\n');
fs.writeFileSync(
  path.join(out, 'inline.html'),
  `<!doctype html><meta charset="utf-8"><style>${css}</style><div id="root"></div><script>window.__samples=${JSON.stringify(samples)};window.__stateSweep=${['--states', '--validation', '--dialogs'].includes(process.argv[3])};window.__validationOnly=${process.argv[3] === '--validation'};window.__dialogsOnly=${process.argv[3] === '--dialogs'};window.__diagnosticsOnly=${process.argv[3] === '--diagnostics'};</script><script type="module">${js.replaceAll('</script', '<\\/script')}</script>`,
);
fs.copyFileSync(path.join(here, 'driver.js'), path.join(out, 'driver.js'));
fs.copyFileSync(path.join(here, 'host.swift'), path.join(out, 'host.swift'));
const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, { encoding: 'utf8', ...options });
  fs.writeFileSync(
    path.join(out, command === 'swiftc' ? 'compile.log' : 'runtime.log'),
    `${result.stdout || ''}\n${result.stderr || ''}`,
  );
  if (result.error || result.status !== 0)
    throw new Error(`${command}: ${result.error || result.status}`);
  return result;
};
run('swiftc', [
  '-module-cache-path',
  path.join(out, 'swift-cache'),
  path.join(out, 'host.swift'),
  '-o',
  path.join(out, 'layout-host'),
]);
for (const name of ['home', 'config', 'data', 'state', 'cache', 'runtime', 'tmp'])
  fs.mkdirSync(path.join(out, name), { mode: 0o700 });
const env = {
  PATH: '/usr/bin:/bin:/usr/sbin:/sbin',
  LANG: 'en_US.UTF-8',
  HOME: path.join(out, 'home'),
  TMPDIR: path.join(out, 'tmp'),
  XDG_CONFIG_HOME: path.join(out, 'config'),
  XDG_DATA_HOME: path.join(out, 'data'),
  XDG_STATE_HOME: path.join(out, 'state'),
  XDG_CACHE_HOME: path.join(out, 'cache'),
  XDG_RUNTIME_DIR: path.join(out, 'runtime'),
};
const host = run(path.join(out, 'layout-host'), [out], { env, timeout: 380000 });
fs.writeFileSync(
  path.join(out, 'process-receipt.json'),
  JSON.stringify(
    {
      status: host.status,
      signal: host.signal,
      waited: true,
      persistent: !host.stdout.includes('persistent=false;'),
      fixture: 'copied React / synthetic bridge, not Tauri production',
    },
    null,
    2,
  ),
);
const results = JSON.parse(fs.readFileSync(path.join(out, 'layout-results.json'), 'utf8'));
const failures = results.filter(
  (r) =>
    r.error ||
    r.invalidBridgeData ||
    r.invalidResponses?.length ||
    r.missingDialogError ||
    (r.capture && !fs.existsSync(path.join(out, r.name + '.png'))) ||
    r.playCalls > 0 ||
    r.scrollWidth > r.width + 1 ||
    r.outside?.length ||
    r.obscuredControls?.length ||
    r.lastControlInView === false ||
    r.focusInsideDialog === false ||
    r.toastOpacity === '0' ||
    r.toastSaveOverlapPx > 0,
);
const hashes = {};
function hashTree(dir) {
  for (const name of fs.readdirSync(dir)) {
    const file = path.join(dir, name);
    if (fs.statSync(file).isDirectory()) hashTree(file);
    else
      hashes[path.relative(desktop, file)] = crypto
        .createHash('sha256')
        .update(fs.readFileSync(file))
        .digest('hex');
  }
}
hashTree(path.join(desktop, 'src'));
hashTree(here);
fs.writeFileSync(path.join(out, 'source-hashes.json'), JSON.stringify(hashes, null, 2));
fs.writeFileSync(
  path.join(out, 'summary.json'),
  JSON.stringify(
    {
      measurements: results.length,
      combinations: results.find((r) => r.name === 'summary')?.combinationsCompleted || 0,
      failures,
    },
    null,
    2,
  ),
);
console.log(JSON.stringify({ out, measurements: results.length, failures: failures.length }));
if (
  failures.length ||
  !host.stdout.includes('persistent=false;') ||
  results.find((r) => r.name === 'summary')?.combinationsCompleted !==
    (['--states', '--validation', '--dialogs'].includes(process.argv[3]) ? 2 : 8)
)
  process.exitCode = 1;
