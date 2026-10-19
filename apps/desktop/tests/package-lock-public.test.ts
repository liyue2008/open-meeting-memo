import fs from 'node:fs';
import { expect, it } from 'vitest';

it('package-lock publishes exact official tarball URLs with pinned integrity', () => {
  const lock = JSON.parse(fs.readFileSync('package-lock.json', 'utf8')) as {
    packages: Record<string, { resolved?: string; version: string; integrity?: string }>;
  };
  let checked = 0;
  for (const [path, entry] of Object.entries(lock.packages)) {
    if (!entry.resolved) continue;
    const name = path.split('node_modules/').at(-1)!;
    expect(entry.resolved, path).toBe(
      `https://registry.npmjs.org/${name}/-/${name.split('/').at(-1)}-${entry.version}.tgz`,
    );
    expect(entry.integrity, path).toMatch(/^sha512-[A-Za-z0-9+/]+={0,2}$/);
    checked++;
  }
  expect(checked).toBeGreaterThan(0);
});
