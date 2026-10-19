import fs from 'node:fs';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { App } from '../src/App';
import { fixtureBridge, sample } from './fixtures';

describe('features/lifecycle banner entry foreground/background pairing', () => {
  for (const state of ['uninitialized', 'unknown'] as const) {
    it(`uses the scoped service entry for ${state}`, async () => {
      const fake = fixtureBridge({ service_plan: () => ({ ...sample('service_plan'), state }) });
      render(<App bridge={fake.bridge} />);
      const button = await screen.findByRole('button', {
        name: state === 'uninitialized' ? '打开首次设置' : '打开服务设置',
      });
      expect(button).toHaveClass('service-entry');
      expect(button.closest('.compatibility-banner')).toBeTruthy();
    });
  }
  it('shares explicit colors and disables native paint for normal, hover and focus states', () => {
    const css = fs.readFileSync('src/styles.css', 'utf8');
    const rule = css.match(
      /\.compatibility-banner \.service-entry,\s*\.compatibility-banner \.service-entry:hover:not\(:disabled\),\s*\.compatibility-banner \.service-entry:focus-visible\s*\{([^}]+)\}/,
    )?.[1];
    expect(rule).toBeDefined();
    expect(rule).toContain('color: var(--ink)');
    expect(rule).toContain('background: var(--surface)');
    expect(rule).toContain('-webkit-appearance: none');
    expect(rule).toContain('appearance: none');
    const hover = css.match(
      /\.compatibility-banner \.service-entry:hover:not\(:disabled\)\s*\{([^}]+)\}/,
    )?.[1];
    expect(hover?.trim()).toBe('border-color: var(--teal);');
  });
});
