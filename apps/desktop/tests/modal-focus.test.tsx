import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { render } from './render';
import { useState } from 'react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Modal } from '../src/components/ui';
import { App } from '../src/App';
import { conflict, fixtureBridge, sample } from './fixtures';

// JSDOM retains disabled focus; WK instead blurs to BODY. Both must recover
// to a usable title, not leave focus on an unavailable control.
const plan = (action: 'start' | 'stop' | 'switch') => ({
  ...sample('service_plan'),
  action,
  expiresAtMs: Date.now() + 60000,
});

describe('components/ui Modal focus during asynchronous service actions', () => {
  it('keeps busy and stale Start failure in the dialog without enabling cancellation or replay', async () => {
    let fail!: () => void;
    const fake = fixtureBridge({
      service_plan: ({ action }) => plan(action),
      service_apply: () =>
        new Promise((_, reject) => {
          fail = () => reject(conflict());
        }),
    });
    render(<App bridge={fake.bridge} />);
    await userEvent.click(screen.getByRole('link', { name: '状态与诊断' }));
    await userEvent.click(await screen.findByRole('button', { name: '启动后台服务' }));
    const dialog = await screen.findByRole('dialog');
    const confirm = within(dialog).getByRole('button', { name: '确认启动' });
    confirm.focus();
    fireEvent.click(confirm);
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    await waitFor(() => expect(document.activeElement).not.toBe(confirm));
    expect(document.activeElement?.matches(':disabled')).toBe(false);
    expect(within(dialog).getByRole('button', { name: '暂不操作' })).toBeDisabled();
    expect(within(dialog).getByRole('button', { name: '关闭对话框' })).toBeDisabled();
    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(dialog).toBeInTheDocument();
    await act(async () => fail());
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('配置已被其他进程修改');
    expect(dialog.contains(document.activeElement)).toBe(true);
    expect(confirm).toBeDisabled();
    await userEvent.click(within(dialog).getByRole('button', { name: '重新检查计划' }));
    expect(confirm).toBeEnabled();
    expect(dialog.contains(document.activeElement)).toBe(true);
    expect(fake.calls.filter((call) => call.command === 'service_apply')).toHaveLength(1);
  });
  it('recovers expired confirmation focus while preserving expiry and explicit recheck', async () => {
    const fake = fixtureBridge({ service_plan: ({ action }) => plan(action) });
    render(<App bridge={fake.bridge} />);
    await userEvent.click(screen.getByRole('link', { name: '状态与诊断' }));
    await userEvent.click(await screen.findByRole('button', { name: '启动后台服务' }));
    const dialog = await screen.findByRole('dialog');
    const confirm = within(dialog).getByRole('button', { name: '确认启动' });
    confirm.focus();
    vi.spyOn(Date, 'now').mockReturnValue(Date.now() + 120000);
    await waitFor(() => expect(confirm).toBeDisabled(), { timeout: 1800 });
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    await waitFor(() => expect(document.activeElement).not.toBe(confirm));
    expect(document.activeElement?.matches(':disabled')).toBe(false);
    expect(within(dialog).getByText(/计划已过期/)).toBeVisible();
    expect(fake.calls.some((call) => call.command === 'service_apply')).toBe(false);
  });
  it('recovers a removed focused control, but preserves textarea selection and unrelated renders', async () => {
    const view = render(
      <Modal title="编辑资料" close={() => undefined}>
        <>
          <textarea aria-label="草稿" defaultValue="abcdef" />
          <button>保存</button>
        </>
      </Modal>,
    );
    const save = screen.getByRole('button', { name: '保存' });
    save.focus();
    view.rerender(
      <Modal title="编辑资料" close={() => undefined}>
        <>
          <textarea aria-label="草稿" defaultValue="abcdef" />
          {null}
        </>
      </Modal>,
    );
    await waitFor(() =>
      expect(screen.getByRole('dialog').contains(document.activeElement)).toBe(true),
    );
    const field = screen.getByRole('textbox') as HTMLTextAreaElement;
    field.focus();
    field.setSelectionRange(1, 4);
    field.scrollTop = 12;
    view.rerender(
      <Modal title="编辑资料" close={() => undefined}>
        <>
          <textarea aria-label="草稿" defaultValue="abcdef" />
          <p>新的校验消息</p>
        </>
      </Modal>,
    );
    await act(async () => undefined);
    expect(field).toHaveFocus();
    expect(field.selectionStart).toBe(1);
    expect(field.selectionEnd).toBe(4);
    expect(field.scrollTop).toBe(12);
  });
  it('does not steal deliberately external focus or change close, Escape and trigger restoration', async () => {
    function Example() {
      const [open, setOpen] = useState(false);
      const [text, setText] = useState('旧');
      return (
        <>
          <button onClick={() => setOpen(true)}>打开</button>
          <button onClick={() => setText('新')}>外部</button>
          {open && (
            <Modal title="编辑" close={() => setOpen(false)}>
              <p>{text}</p>
              <button>保存</button>
            </Modal>
          )}
        </>
      );
    }
    render(<Example />);
    const trigger = screen.getByRole('button', { name: '打开' });
    await userEvent.click(trigger);
    const dialog = screen.getByRole('dialog');
    within(dialog).getByRole('button', { name: '保存' }).focus();
    const external = screen.getByRole('button', { name: '外部' });
    external.focus();
    fireEvent.click(external);
    await act(async () => undefined);
    expect(external).toHaveFocus();
    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await waitFor(() => expect(trigger).toHaveFocus());
    await userEvent.click(trigger);
    await userEvent.click(screen.getByRole('button', { name: '关闭对话框' }));
    await waitFor(() => expect(trigger).toHaveFocus());
  });
});
