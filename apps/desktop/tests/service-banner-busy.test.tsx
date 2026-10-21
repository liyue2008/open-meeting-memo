import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../src/App';
import { fixtureBridge, sample, taskFixture } from './fixtures';

describe('ServiceBanner separates service health from protected operations', () => {
  it('does not warn globally for a compatible running service with a reader, but retains stop protection', async () => {
    const fake = fixtureBridge({
      service_plan: ({ action }) => ({
        ...sample('service_plan'),
        action,
        state: 'busy',
        reasons: ['service_busy'],
        canApply: false,
        planToken: null,
        expiresAtMs: null,
      }),
    });
    render(<App bridge={fake.bridge} />);
    await waitFor(() => expect(fake.calls.some((c) => c.command === 'service_plan')).toBe(true));
    await screen.findByText('当前未录音');
    expect(
      screen.queryByRole('status', { name: '本机首次设置与服务状态' }),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('link', { name: '状态与诊断' }));
    await userEvent.click(await screen.findByRole('button', { name: '停止后台服务' }));
    expect(await within(screen.getByRole('dialog')).findByText(/文件读取占用/)).toBeVisible();
    expect(screen.getByRole('button', { name: '确认停止' })).toBeDisabled();
    expect(fake.calls.some((c) => c.command === 'service_apply')).toBe(false);
  });
  for (const [state, loadedLabel] of [
    ['stopped', '后台服务：已停止。'],
    ['unknown', '后台服务：状态未知。'],
    ['incompatible', '后台服务：版本不兼容。'],
  ] as const) {
    it(`retains the global warning for ${state}`, async () => {
      const fake = fixtureBridge({ service_plan: () => ({ ...sample('service_plan'), state }) });
      render(<App bridge={fake.bridge} />);
      const banner = screen.getByRole('status', { name: '本机首次设置与服务状态' });
      expect(await within(banner).findByText(loadedLabel)).toBeVisible();
      expect(within(banner).getByRole('button', { name: '打开服务设置' })).toBeVisible();
    });
  }
  it('does not hide incompatible busy plans', async () => {
    const fake = fixtureBridge({
      service_plan: () => ({
        ...sample('service_plan'),
        state: 'busy',
        reasons: ['unowned_service'],
        compatibility: { ...sample('service_plan').compatibility, writesAllowed: false },
      }),
    });
    render(<App bridge={fake.bridge} />);
    const banner = screen.getByRole('status', { name: '本机首次设置与服务状态' });
    expect(await within(banner).findByText('后台服务：停止或切换暂受保护。')).toBeVisible();
    expect(within(banner).getByRole('button', { name: '打开服务设置' })).toBeVisible();
  });
  it('labels the queue count as attention including failed and blocked, not running work', async () => {
    const fake = fixtureBridge({
      task_list: () => ({
        ...sample('task_list'),
        items: [taskFixture(), { ...taskFixture(), status: 'blocked' }],
      }),
    });
    render(<App bridge={fake.bridge} />);
    expect((await screen.findByText('2 待关注')).closest('[title]')).toHaveAttribute(
      'title',
      expect.stringContaining('失败、阻塞'),
    );
  });
});
