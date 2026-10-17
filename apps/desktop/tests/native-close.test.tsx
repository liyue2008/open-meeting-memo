import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { App } from '../src/App';
import { fixtureBridge, sample } from './fixtures';
const native = vi.hoisted(() => ({
  listener: null as null | ((event: { preventDefault: () => void }) => void),
  destroy: vi.fn<() => Promise<void>>(),
  unlisten: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => true,
  invoke: () => {
    throw new Error('No actual invoke in DOM-only tests');
  },
}));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onCloseRequested: async (callback: typeof native.listener) => {
      native.listener = callback;
      return native.unlisten;
    },
    destroy: native.destroy,
  }),
}));
describe('Native window close draft guard (DOM fixture, not permission proof)', () => {
  it('prevents dirty native close, saves CAS then destroys only the current window', async () => {
    native.destroy.mockResolvedValue();
    const fake = fixtureBridge({
      config_validate: () => ({ ...sample('config_validate'), valid: true }),
      config_save: (p) => ({
        ...sample('config_get'),
        values: { ...sample('config_get').values, ...p.patch },
      }),
    });
    const { unmount } = render(<App bridge={fake.bridge} />);
    await userEvent.click(screen.getByRole('link', { name: '设置' }));
    const input = await screen.findByLabelText('Pi 模型');
    await userEvent.type(input, 'changed');
    await waitFor(() => expect(native.listener).not.toBeNull());
    const preventDefault = vi.fn();
    act(() => native.listener?.({ preventDefault }));
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(native.destroy).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: '保存并离开' }));
    await waitFor(() => expect(native.destroy).toHaveBeenCalledOnce());
    expect(fake.calls.some((c) => c.command === 'config_save')).toBe(true);
    unmount();
    expect(native.unlisten).toHaveBeenCalled();
  });
  it('does not prevent an unmodified close', async () => {
    render(<App bridge={fixtureBridge().bridge} />);
    await screen.findByText('没有符合条件的会议');
    const preventDefault = vi.fn();
    act(() => native.listener?.({ preventDefault }));
    expect(preventDefault).not.toHaveBeenCalled();
  });
});
