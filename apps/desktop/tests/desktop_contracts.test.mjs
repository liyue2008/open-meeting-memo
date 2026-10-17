import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const dir = path.join(root, 'contracts/desktop/v1');
const schema = JSON.parse(fs.readFileSync(path.join(dir, 'schema.json'), 'utf8'));
const ajv = new Ajv2020({ strict: true, allErrors: true });
ajv.addKeyword('x-commands'); ajv.addKeyword('x-budgets'); ajv.addKeyword('x-service-capability');
addFormats(ajv);
const validate = ajv.compile(schema);
let assertions = 0;
function check(value, valid) {
  assert.equal(validate(value), valid, JSON.stringify(validate.errors)); assertions++;
}
const exchanges = fs.readdirSync(path.join(dir, 'examples')).filter(n => n.endsWith('.json')).flatMap(n => {
  const value = JSON.parse(fs.readFileSync(path.join(dir, 'examples', n), 'utf8'));
  return Array.isArray(value) ? value : [value];
});
assert.deepEqual(new Set(exchanges.map(e => e.command)), new Set(Object.keys(schema['x-commands'])));
for (const e of exchanges) {
  check(e, true);
  check({ ...e, injected: true }, false);
  check({ ...e, request: { ...e.request, injected: true } }, false);
  check({ ...e, response: { ...e.response, injected: true } }, false);
  check({ ...e, request: { ...e.request, schemaVersion: 2 } }, false);
  check({ ...e, request: { ...e.request, requestId: '../private/path' } }, false);
}
// Check every named DTO's closed-object, enum, nullable and scalar boundary rules.
for (const [name, definition] of Object.entries(schema.$defs)) {
  const checkDef = ajv.compile({ $schema: schema.$schema, $ref: `${schema.$id}#/$defs/${name}` });
  if (definition.type === 'object') { assert.equal(checkDef({ injected: true }), false, name); assertions++; }
  if (definition.enum) {
    for (const value of definition.enum) { assert.equal(checkDef(value), true, name); assertions++; }
    assert.equal(checkDef('not-a-valid-enum-member'), false, name); assertions++;
  }
  if (definition.anyOf?.some(v => v.type === 'null')) { assert.equal(checkDef(null), true, name); assertions++; }
}
const meeting = structuredClone(exchanges.find(e => e.command === 'meeting_update'));
meeting.request.payload.metadata.title = 'x'.repeat(257); check(meeting, false);
meeting.request.payload.metadata.title = 'Valid fixture';
meeting.request.payload.metadata.note = 'x'.repeat(16385); check(meeting, false);
meeting.request.payload.metadata.note = '';
meeting.request.payload.metadata.tags = ['same', 'same']; check(meeting, false);
const page = structuredClone(exchanges.find(e => e.command === 'meeting_list'));
for (const size of [0, 101, 1.5, Number.MAX_SAFE_INTEGER + 1]) { page.request.payload.pageSize = size; check(page, false); }
page.request.payload.pageSize = 25; page.request.payload.filter.query = 'x'.repeat(513); check(page, false);
const config = structuredClone(exchanges.find(e => e.command === 'config_save'));
config.request.payload.patch = { autoRecord: true }; check(config, false);
config.request.payload.patch = { piNodeExecutable: null }; check(config, true);
config.request.payload.patch = { piNodeExecutable: '/fixture/bin/node' }; check(config, true);
const media = structuredClone(exchanges.find(e => e.command === 'media_open'));
media.request.payload.path = '/private/arbitrary.wav'; check(media, false);
media.response.data.url = 'file:///private/arbitrary.wav'; delete media.request.payload.path; check(media, false);
const admission = structuredClone(exchanges.find(e => e.command === 'config_save'));
for (const mode of ['service', 'offline']) { admission.request.payload.admission.mode = mode; check(admission, true); }
for (const field of ['ownerGeneration', 'profileId', 'token', 'expiresAtMs', 'mode']) {
  const missing = structuredClone(admission); delete missing.request.payload.admission[field]; check(missing, false);
}
admission.request.payload.admission.mode = 'unknown'; check(admission, false);
admission.request.payload.admission.mode = 'offline'; admission.request.payload.admission.expiresAtMs = -1; check(admission, false);
const service = JSON.parse(fs.readFileSync(path.join(dir, 'service-capability.examples.json')));
for (const [name, value] of [['ServiceCapabilityRequest', service.request.desktop_capability], ['ServiceCapabilityResponse', service.response.data]]) {
  const validateService = ajv.compile({ $schema: schema.$schema, $ref: `${schema.$id}#/$defs/${name}` });
  assert.equal(validateService(value), true, JSON.stringify(validateService.errors)); assertions++;
  assert.equal(validateService({ ...value, injected: true }), false); assertions++;
}
assert.equal(service.request.command, schema['x-service-capability'].command);
assert.equal(service.request.desktop_capability.intent, 'read_only');
assert.equal(service.response.data.compatibility.serviceGeneration, service.response.service_instance);
assert.equal(service.response.data.compatibility.writeAdmission, null);
const conf = JSON.parse(fs.readFileSync(path.join(root, 'apps/desktop/src-tauri/tauri.conf.json')));
assert.equal(conf.app.security.assetProtocol.enable, false);
assert.equal(conf.bundle.macOS.minimumSystemVersion, '15.5');
assert.ok(!conf.app.security.csp.includes('unsafe-inline'));
assert.ok(!conf.app.security.csp.includes('unsafe-eval'));
assert.ok(!conf.app.security.csp.includes('https:'));
const cap = JSON.parse(fs.readFileSync(path.join(root, 'apps/desktop/src-tauri/capabilities/main.json')));
assert.deepEqual(cap.permissions, []);
assert.equal(cap.local, true);
assert.equal(cap.remote, undefined);
assert.deepEqual(cap.windows, ['main']);
console.log(`Desktop contract: ${exchanges.length} shared exchanges, ${assertions} schema assertions; deny-default ACL and offline CSP verified.`);
