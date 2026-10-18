import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../src/App';
import type { ServiceAction, ServicePlan, ServicePlanReason } from '../src/contracts.generated';
import { conflict, fixtureBridge, sample } from './fixtures';

const plan = (action: ServiceAction, patch: Partial<ServicePlan> = {}): ServicePlan => ({
  ...sample('service_plan'),
  action,
  expiresAtMs: Date.now() + 60000,
  ...patch,
});
const services = async () => {
  await userEvent.click(screen.getByRole('link', { name: '状态与诊断' }));
  return screen.findByRole('region', { name: '首次设置与后台服务' });
};
describe('features/lifecycle frozen service recovery UI', () => {
  it('keeps unknown saved preferences and recording state unknown, with no guessed toggle', async () => {
    const fake = fixtureBridge({
      record_status: () => ({
        ...sample('record_status'),
        state: 'unknown',
        autoRecord: null,
        recordingConsent: null,
      }),
      config_get: () => ({
        ...sample('config_get'),
        initialized: true,
        autoRecord: null,
        recordingConsent: null,
      }),
    });
    render(<App bridge={fake.bridge} />);
    expect(await screen.findByText(/已保存的自动录音偏好：未知/)).toBeVisible();
    expect(screen.getByRole('button', { name: '自动录音偏好未知' })).toBeDisabled();
    expect(screen.queryByText(/当前未录音/)).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('link', { name: '设置' }));
    expect(await screen.findByText(/自动录音：未知 · 录音同意：未知/)).toBeVisible();
    expect(screen.getByText('模型与配置结构状态未知')).toBeVisible();
    expect(
      [...document.querySelectorAll('.settings-grid fieldset')].every(
        (field) => (field as HTMLFieldSetElement).disabled,
      ),
    ).toBe(true);
    expect(
      fake.calls.some((c) => c.command === 'record_auto_set' || c.command === 'config_save'),
    ).toBe(false);
  });
  it('separates a known saved preference from an unknown read-only recording service', async () => {
    const status = sample('record_status');
    const fake = fixtureBridge({
      record_status: () => ({
        ...status,
        state: 'unknown',
        autoRecord: true,
        recordingConsent: true,
        compatibility: {
          ...status.compatibility,
          writesAllowed: false,
          reason: 'profile_requires_takeover',
        },
      }),
    });
    render(<App bridge={fake.bridge} />);
    expect(await screen.findByText('录音状态未知')).toBeVisible();
    expect(screen.getByText(/已保存的自动录音偏好：已启用/)).toBeVisible();
    expect(screen.getByRole('button', { name: '关闭自动录音' })).toBeDisabled();
    expect(screen.getByText(/当前服务未授权写入，保留既有偏好/)).toBeVisible();
  });
  it('does not treat a null automatic-recording mutation result as confirmed false', async () => {
    const fake = fixtureBridge({
      record_status: () => ({
        ...sample('record_status'),
        autoRecord: true,
        recordingConsent: true,
      }),
      record_auto_set: () => ({
        ...sample('record_auto_set'),
        autoRecord: null,
        recordingConsent: null,
      }),
    });
    render(<App bridge={fake.bridge} />);
    await userEvent.click(await screen.findByRole('button', { name: '关闭自动录音' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('自动录音尚未关闭');
  });
  it('adopts without starting, then keeps a failed Start stopped and retryable', async () => {
    let adopted = false,
      attempts = 0;
    const fake = fixtureBridge({
      service_plan: ({ action }) =>
        plan(
          action,
          adopted
            ? { state: 'stopped' }
            : {
                state: 'stopped',
                reasons: ['profile_requires_takeover'],
                canApply: action === 'switch',
                effects: [
                  'Keep configuration, consent and database bytes unchanged; do not start a service',
                ],
                guidance: 'Adopt only; then separately inspect Start.',
              },
        ),
      service_apply: () => {
        if (!adopted) {
          adopted = true;
          return { ...sample('service_apply'), action: 'switch', state: 'stopped' };
        }
        if (++attempts === 1) throw conflict();
        return { ...sample('service_apply'), action: 'start', state: 'running' };
      },
      config_get: () => ({
        ...sample('config_get'),
        initialized: true,
        autoRecord: true,
        recordingConsent: true,
      }),
    });
    render(<App bridge={fake.bridge} />);
    await services();
    await userEvent.click(screen.getByRole('button', { name: '安全切换服务版本' }));
    const adoption = await screen.findByRole('dialog', {
      name: '采用资料库并更新停机登记（不启动）',
    });
    expect(within(adoption).getByText(/Keep configuration/)).toBeVisible();
    await userEvent.click(within(adoption).getByRole('button', { name: '确认采用（不启动）' }));
    expect(
      await screen.findByText(/采用完成，服务仍已停止；下一步请另点“启动后台服务”/),
    ).toBeVisible();
    expect(fake.calls.filter((c) => c.command === 'service_apply')).toHaveLength(1);
    await userEvent.click(screen.getByRole('button', { name: '启动后台服务' }));
    let start = await screen.findByRole('dialog', { name: '启动后台服务' });
    expect(within(start).getByText(/启动后按既有自动录音偏好与同意设置运行/)).toBeVisible();
    await userEvent.click(within(start).getByRole('button', { name: '确认启动' }));
    expect(await within(start).findByRole('alert')).toBeVisible();
    expect(within(start).getByRole('button', { name: '确认启动' })).toBeDisabled();
    await userEvent.click(within(start).getByRole('button', { name: '重新检查计划' }));
    await waitFor(() =>
      expect(within(start).getByRole('button', { name: '确认启动' })).toBeEnabled(),
    );
    fireEvent.click(within(start).getByRole('button', { name: '确认启动' }));
    fireEvent.click(within(start).getByRole('button', { name: '确认启动' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(attempts).toBe(2);
  });
  it.each([
    'external_stop_required',
    'external_unload_required',
    'registration_review_required',
    'incompatible_schema',
  ] as ServicePlanReason[])(
    'shows %s guidance safely, next to the disabled confirmation',
    async (reason) => {
      const guidance =
        'VERIFY ORIGINAL OMM; keep plist; /bin/launchctl bootout gui/501/org.openmeetingmemo.service <script>not executed</script>';
      const fake = fixtureBridge({
        service_plan: ({ action }) =>
          plan(action, {
            state: 'unknown',
            reasons: [reason],
            canApply: false,
            planToken: null,
            expiresAtMs: null,
            guidance,
            effects: ['No files changed'],
          }),
      });
      render(<App bridge={fake.bridge} />);
      await services();
      await userEvent.click(screen.getByRole('button', { name: '安全切换服务版本' }));
      const dialog = await screen.findByRole('dialog');
      expect(within(dialog).getByLabelText('原生端人工处理指导')).toHaveValue(guidance);
      expect(within(dialog).getByRole('button', { name: '复制人工处理指导' })).toBeEnabled();
      expect(within(dialog).getByRole('button', { name: '确认安全切换' })).toBeDisabled();
      expect(within(dialog).getByText(/不能执行：/)).toBeVisible();
      expect(dialog.querySelector('script')).toBeNull();
      expect(fake.calls.some((c) => c.command === 'service_apply')).toBe(false);
    },
  );
  it('keeps a copy failure inside the current modal and permits manual text selection', async () => {
    const fake = fixtureBridge({
      service_plan: ({ action }) =>
        plan(action, {
          canApply: false,
          guidance: 'Read and manually verify this synthetic guidance.',
          reasons: ['registration_review_required'],
        }),
    });
    render(<App bridge={fake.bridge} />);
    await services();
    await userEvent.click(screen.getByRole('button', { name: '安全切换服务版本' }));
    const dialog = await screen.findByRole('dialog');
    const descriptor = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: undefined });
    try {
      await userEvent.click(within(dialog).getByRole('button', { name: '复制人工处理指导' }));
      expect(await within(dialog).findByRole('alert')).toHaveTextContent('请在指导框内全选并复制');
      expect(within(dialog).getByLabelText('原生端人工处理指导')).toHaveAttribute('readonly');
    } finally {
      if (descriptor) Object.defineProperty(navigator, 'clipboard', descriptor);
      else delete (navigator as unknown as Record<string, unknown>).clipboard;
    }
  });
  it('explains an expired plan and a successful explicit recheck', async () => {
    let fresh = false;
    const fake = fixtureBridge({
      service_plan: ({ action }) =>
        plan(action, { expiresAtMs: Date.now() + (fresh ? 60000 : -1) }),
    });
    render(<App bridge={fake.bridge} />);
    await services();
    await userEvent.click(screen.getByRole('button', { name: '启动后台服务' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/计划已过期/)).toBeVisible();
    fresh = true;
    await userEvent.click(within(dialog).getByRole('button', { name: '重新检查计划' }));
    expect(await within(dialog).findByText(/已重新检查计划/)).toBeVisible();
  });
});
