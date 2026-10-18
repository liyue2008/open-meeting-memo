import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Button, DEFAULT_THEME, useMantineTheme } from '@mantine/core';
import { describe, expect, it, vi } from 'vitest';
import { systemDark } from './setup';
import { AppearanceProvider, useAppearance } from '../src/app/preferences';

function Probe() {
  const appearance = useAppearance();
  const theme = useMantineTheme();
  return (
    <>
      <output aria-label="当前外观">{appearance.theme}</output>
      <Button onClick={() => appearance.setTheme('system')}>跟随系统</Button>
      <Button onClick={() => appearance.setTheme('dark')}>深色</Button>
      <output aria-label="默认主题">
        {String(
          theme.primaryColor === DEFAULT_THEME.primaryColor &&
            theme.fontFamily === DEFAULT_THEME.fontFamily &&
            JSON.stringify(theme.colors) === JSON.stringify(DEFAULT_THEME.colors) &&
            JSON.stringify(theme.spacing) === JSON.stringify(DEFAULT_THEME.spacing) &&
            theme.defaultRadius === DEFAULT_THEME.defaultRadius,
        )}
      </output>
    </>
  );
}
function mount() {
  return render(
    <AppearanceProvider>
      <Probe />
    </AppearanceProvider>,
  );
}

describe('app/preferences Mantine official appearance compatibility', () => {
  it('uses the unmodified default theme and no runtime style elements', () => {
    mount();
    expect(screen.getByLabelText('默认主题')).toHaveTextContent('true');
    expect(document.querySelector('style')).toBeNull();
    expect(localStorage.getItem('omm.theme')).toBeNull();
    expect(localStorage.getItem('mantine-color-scheme-value')).toBeNull();
  });
  it('maps the existing system preference to auto and follows system changes without rewriting it', async () => {
    localStorage.setItem('omm.theme', 'system');
    mount();
    expect(screen.getByLabelText('当前外观')).toHaveTextContent('system');
    act(() => systemDark(true));
    await waitFor(() => expect(document.documentElement.dataset.mantineColorScheme).toBe('dark'));
    expect(localStorage.getItem('omm.theme')).toBe('system');
    act(() => systemDark(false));
    await waitFor(() => expect(document.documentElement.dataset.mantineColorScheme).toBe('light'));
  });
  it('keeps the existing key/spelling and injects no transition style on explicit changes', () => {
    mount();
    fireEvent.click(screen.getByRole('button', { name: '深色' }));
    expect(localStorage.getItem('omm.theme')).toBe('dark');
    expect(document.querySelector('style')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '跟随系统' }));
    expect(localStorage.getItem('omm.theme')).toBe('system');
    expect(document.querySelector('style')).toBeNull();
  });
  it('accepts cross-window updates without writing them back or overwriting unknown values', async () => {
    mount();
    localStorage.setItem('omm.theme', 'dark');
    const write = vi.spyOn(Storage.prototype, 'setItem');
    act(() =>
      window.dispatchEvent(new StorageEvent('storage', { key: 'omm.theme', newValue: 'dark' })),
    );
    await waitFor(() => expect(screen.getByLabelText('当前外观')).toHaveTextContent('dark'));
    expect(write).not.toHaveBeenCalled();
    write.mockRestore();
    localStorage.setItem('omm.theme', 'future-theme');
    act(() =>
      window.dispatchEvent(
        new StorageEvent('storage', { key: 'omm.theme', newValue: 'future-theme' }),
      ),
    );
    expect(localStorage.getItem('omm.theme')).toBe('future-theme');
    expect(screen.getByLabelText('当前外观')).toHaveTextContent('dark');
  });
});
