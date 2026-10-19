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
  it('uses Mantine Button default variant without a competing legacy skin', async () => {
    const fake = fixtureBridge({
      service_plan: () => ({ ...sample('service_plan'), state: 'unknown' }),
    });
    render(<App bridge={fake.bridge} />);
    const button = await screen.findByRole('button', { name: '打开服务设置' });
    expect(button).toHaveClass('mantine-Button-root');
    expect(button).toHaveAttribute('data-variant', 'default');
    const css = fs.readFileSync('src/styles.css', 'utf8');
    expect(css).not.toMatch(/service-entry[^}]*background|service-entry[^}]*color:/);
  });
});
