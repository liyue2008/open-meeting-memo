import { execFileSync } from 'node:child_process';
import { expect, it } from 'vitest';

it('bridge/validators production Vite bundle preserves strict runtime validation', () => {
  expect(
    execFileSync(process.execPath, ['scripts/test-production-validators.mjs'], {
      cwd: process.cwd(),
      timeout: 30000,
      encoding: 'utf8',
    }),
  ).toContain('Unicode, deep equality and invalid DTO cases passed');
}, 35000);
