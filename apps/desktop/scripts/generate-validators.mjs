// Build-only schema compilation: production CSP does not permit eval/Function.
import fs from 'node:fs';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
import standalone from 'ajv/dist/standalone/index.js';
const schema = JSON.parse(
  fs.readFileSync(new URL('../../../contracts/desktop/v1/schema.json', import.meta.url), 'utf8'),
);
const ajv = new Ajv2020({
  strict: true,
  code: { source: true, esm: true },
  formats: { 'date-time': true, uuid: true, date: true },
});
addFormats(ajv);
ajv.addKeyword('x-commands');
ajv.addKeyword('x-budgets');
ajv.addKeyword('x-service-capability');
ajv.addSchema(schema);
const exports = Object.fromEntries(
  Object.keys(schema['x-commands']).flatMap((command) =>
    ['request', 'response'].map((kind) => {
      const name = `${command}_${kind}`;
      ajv.addSchema({ $id: name, $ref: `${schema.$id}#/$defs/${name}` });
      return [name, name];
    }),
  ),
);
let code =
  '// Generated from frozen desktop/v1 schema. Run npm run validators:generate. No runtime eval.\n' +
  standalone(ajv, exports);
// Ajv emits CommonJS runtime references even in ESM mode. Give Vite static ESM imports.
code = code
  .replace(/require\("ajv\/dist\/runtime\/ucs2length"\)\.default/g, 'ucs2length')
  .replace(/require\("ajv-formats\/dist\/formats"\)\.fullFormats/g, 'fullFormats')
  .replace(/require\("ajv\/dist\/runtime\/equal"\)\.default/g, 'equal');
code =
  'import ucs2lengthRuntime from "ajv/dist/runtime/ucs2length.js";\nimport { fullFormats } from "ajv-formats/dist/formats.js";\nimport equalRuntime from "ajv/dist/runtime/equal.js";\n' +
  '// Node-compatible bundlers preserve the CommonJS exports object; source test loaders may unwrap it.\n' +
  'function runtimeFunction(value, name) {\n' +
  '  const fn = typeof value === "function" ? value : value?.default;\n' +
  '  if (typeof fn !== "function") throw new TypeError(`Invalid AJV runtime: ${name}`);\n' +
  '  return fn;\n' +
  '}\n' +
  'const ucs2length = runtimeFunction(ucs2lengthRuntime, "ucs2length");\n' +
  'const equal = runtimeFunction(equalRuntime, "equal");\n' +
  code;
if (code.includes('require('))
  throw new Error(
    `Unexpected standalone runtime requirement: ${code.match(/require\([^)]*\)(?:\.\w+)?/g)?.join(', ')}`,
  );
const output = new URL('../src/bridge/validators.generated.js', import.meta.url);
const declaration =
  Object.keys(exports)
    .map((name) => `export function ${name}(value: unknown): boolean;`)
    .join('\n') + '\n';
if (process.argv.includes('--check')) {
  if (
    fs.readFileSync(output, 'utf8') !== code ||
    fs.readFileSync(new URL('../src/bridge/validators.generated.d.ts', import.meta.url), 'utf8') !==
      declaration
  )
    throw new Error('Renderer validator drift');
} else {
  fs.writeFileSync(output, code);
  fs.writeFileSync(
    new URL('../src/bridge/validators.generated.d.ts', import.meta.url),
    declaration,
  );
}
