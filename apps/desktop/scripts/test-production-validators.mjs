// Exercise emitted browser code, not Vitest's source-level CommonJS interop.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { build } from 'vite';
const app = fileURLToPath(new URL('../', import.meta.url));
const output = fs.mkdtempSync(path.join(os.tmpdir(), 'omm-production-validators-'));
try {
  await build({
    root: app,
    configFile: path.join(app, 'vite.config.ts'),
    logLevel: 'error',
    build: {
      outDir: output,
      emptyOutDir: false,
      minify: true,
      lib: {
        entry: path.join(app, 'src/bridge/validators.generated.js'),
        formats: ['es'],
        fileName: () => 'validators.mjs',
      },
    },
  });
  const validators = await import(pathToFileURL(path.join(output, 'validators.mjs')).href);
  const directory = fileURLToPath(
    new URL('../../../contracts/desktop/v1/examples/', import.meta.url),
  );
  const exchanges = fs
    .readdirSync(directory)
    .filter((name) => name.endsWith('.json'))
    .flatMap((name) => {
      const example = JSON.parse(fs.readFileSync(path.join(directory, name), 'utf8'));
      return Array.isArray(example) ? example : [example];
    })
    .filter((example) => example.command && example.request && example.response);
  for (const example of exchanges) {
    for (const kind of ['request', 'response'])
      assert.equal(
        validators[`${example.command}_${kind}`](example[kind]),
        true,
        `${example.command} ${kind}`,
      );
  }
  const example = (command) => structuredClone(exchanges.find((item) => item.command === command));
  const info = example('desktop_info').response;
  info.data.appVersion = '😀'.repeat(64);
  assert.equal(validators.desktop_info_response(info), true, '64 Unicode code points are valid');
  info.data.appVersion += '😀';
  assert.equal(
    validators.desktop_info_response(info),
    false,
    '65 Unicode code points exceed the bound',
  );
  const batch = example('meeting_trash').request;
  batch.payload.meetings.push(structuredClone(batch.payload.meetings[0]));
  assert.equal(
    validators.meeting_trash_request(batch),
    false,
    'structurally equal meeting revisions are duplicates',
  );
  batch.payload.meetings[1].expectedRevision++;
  assert.equal(
    validators.meeting_trash_request(batch),
    true,
    'different revisions are not structurally equal',
  );
  for (const mutate of [
    (value) => {
      value.extra = true;
    },
    (value) => {
      value.schemaVersion = 2;
    },
    (value) => {
      value.payload.path = '/etc/passwd';
    },
  ]) {
    const request = example('desktop_info').request;
    mutate(request);
    assert.equal(validators.desktop_info_request(request), false, 'invalid DTO refused');
  }
  console.log(
    `Production Vite validator bundle: ${exchanges.length} exchanges; Unicode, deep equality and invalid DTO cases passed.`,
  );
} finally {
  fs.rmSync(output, { recursive: true, force: true });
}
