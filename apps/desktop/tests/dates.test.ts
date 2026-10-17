import { describe, expect, it } from 'vitest';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import ts from 'typescript';
import { dateRange, localDay } from '../src/app/dates';
describe('app/dates local calendar ranges', () => {
  const now = new Date(2026, 9, 18, 16, 30); // Sunday
  it('defaults seven inclusive local days and Monday-start current week', () => {
    expect(dateRange('7', now)?.fromLocalDate).toBe('2026-10-12');
    expect(dateRange('7', now)?.toLocalDateExclusive).toBe('2026-10-19');
    expect(dateRange('week', now)?.fromLocalDate).toBe('2026-10-12');
    expect(dateRange('3', now)?.fromLocalDate).toBe('2026-10-16');
    expect(dateRange('today', now)?.fromLocalDate).toBe(localDay(now));
    expect(dateRange('all', now)).toBeNull();
  });
  it('validates leap dates, ordering and inclusive custom end', () => {
    expect(dateRange('custom', now, '2024-02-29', '2024-02-29')?.toLocalDateExclusive).toBe(
      '2024-03-01',
    );
    for (const [from, to] of [
      ['2025-02-29', '2025-03-01'],
      ['2026-11-10', '2026-11-01'],
      ['', '2026-01-01'],
      ['2026-04-31', '2026-05-02'],
    ])
      expect(() => dateRange('custom', now, from, to)).toThrow();
  });
  it('uses calendar arithmetic across spring/fall DST and year boundaries in independent zones', () => {
    const source = fs.readFileSync(`${process.cwd()}/src/app/dates.ts`, 'utf8');
    const code = ts.transpileModule(source, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
    }).outputText;
    const assertions = `
      import assert from 'node:assert/strict';
      const spring = dateRange('today', new Date(2026,2,8,12));
      const fall = dateRange('today', new Date(2026,10,1,12));
      assert.equal(spring.toUtcMsExclusive - spring.fromUtcMs, process.env.TZ === 'America/New_York' ? 23*3600000 : 24*3600000);
      assert.equal(fall.toUtcMsExclusive - fall.fromUtcMs, process.env.TZ === 'America/New_York' ? 25*3600000 : 24*3600000);
      assert.equal(dateRange('7', new Date(2026,0,2,12)).fromLocalDate, '2025-12-27');
      assert.equal(dateRange('week', new Date(2026,0,1,12)).fromLocalDate, '2025-12-29');
    `;
    for (const TZ of ['America/New_York', 'Asia/Shanghai'])
      expect(() =>
        execFileSync(process.execPath, ['--input-type=module', '-e', code + assertions], {
          env: { ...process.env, TZ },
          timeout: 10000,
        }),
      ).not.toThrow();
  });
});
