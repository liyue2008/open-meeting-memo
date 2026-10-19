import fs from 'node:fs';
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { verifyProjectNotice } from '../scripts/collect-notices.mjs';

const location = 'node_modules/react-remove-scroll-bar';
const entry = JSON.parse(fs.readFileSync(new URL('../package-lock.json', import.meta.url), 'utf8'))
  .packages[location];
const metadata = JSON.parse(
  fs.readFileSync(
    new URL('../node_modules/react-remove-scroll-bar/package.json', import.meta.url),
    'utf8',
  ),
);
const text = fs.readFileSync(
  new URL('../licenses/react-remove-scroll-bar-project-MIT.LICENSE', import.meta.url),
  'utf8',
);

describe('scripts/collect-notices exact upstream-project notice mapping', () => {
  it('accepts only the reviewed 2.3.8 package and preserves the original text', () => {
    assert.equal(verifyProjectNotice(location, entry, metadata, text), text);
  });
  for (const candidate of [
    {
      name: 'another missing-license package',
      location: 'node_modules/another-package',
      entry,
      metadata,
      text,
    },
    {
      name: 'package name drift',
      location,
      entry,
      metadata: { ...metadata, name: 'another-package' },
      text,
    },
    { name: 'lock version drift', location, entry: { ...entry, version: '2.3.9' }, metadata, text },
    {
      name: 'installed version drift',
      location,
      entry,
      metadata: { ...metadata, version: '2.3.9' },
      text,
    },
    {
      name: 'integrity drift',
      location,
      entry: { ...entry, integrity: 'sha512-unverified' },
      metadata,
      text,
    },
    { name: 'lock license drift', location, entry: { ...entry, license: 'ISC' }, metadata, text },
    {
      name: 'installed license drift',
      location,
      entry,
      metadata: { ...metadata, license: 'ISC' },
      text,
    },
    {
      name: 'repository drift',
      location,
      entry,
      metadata: { ...metadata, repository: 'https://example.invalid/other' },
      text,
    },
    { name: 'license text drift', location, entry, metadata, text: text.replace('MIT', 'ISC') },
  ]) {
    it(`rejects ${candidate.name}`, () => {
      assert.throws(
        () =>
          verifyProjectNotice(
            candidate.location,
            candidate.entry,
            candidate.metadata,
            candidate.text,
          ),
        /Unverified missing-license notice/,
      );
    });
  }
});
