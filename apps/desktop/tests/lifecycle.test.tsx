import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../src/App';
import type { ServiceAction, ServicePlan } from '../src/contracts.generated';
import { fixtureBridge, sample, conflict } from './fixtures';

const plan = (action: ServiceAction, patch: Partial<ServicePlan> = {}): ServicePlan => ({
  ...sample('service_plan'),
  action,
  expiresAtMs: Date.now() + 60000,
  ...patch,
});
const lifecycleWrites = (fake: ReturnType<typeof fixtureBridge>) =>
  fake.calls.filter(
    (call) => call.command === 'profile_initialize' || call.command === 'service_apply',
  );
async function openServices() {
  await userEvent.click(screen.getByRole('link', { name: '状态与诊断' }));
  return screen.findByRole('region', { name: '首次设置与后台服务' });
}

describe('features/lifecycle explicit first use and protected service plans', () => {
  it('observes an empty profile without mutation, initializes only on confirmation, then separately starts and shows missing dependencies', async () => {
    let initialized = false;
    let running = false;
    const fake = fixtureBridge({
      service_plan: ({ action }) =>
        plan(
          action,
          initialized
            ? { state: running ? 'running' : 'stopped' }
            : {
                state: 'uninitialized',
                canInitialize: true,
                canApply: false,
                planToken: null,
                expiresAtMs: null,
                reasons: ['profile_not_initialized'],
              },
        ),
      profile_initialize: () => {
        initialized = true;
        return {
          ...sample('config_get'),
          initialized: true,
          autoRecord: false,
          recordingConsent: false,
        };
      },
      config_get: () => ({
        ...sample('config_get'),
        initialized,
        autoRecord: false,
        recordingConsent: false,
      }),
      service_apply: () => {
        running = true;
        return { ...sample('service_apply'), action: 'start', state: 'running' };
      },
      diagnostics_get: () => ({
        ...sample('diagnostics_get'),
        items: [
          {
            key: 'pi',
            state: 'unavailable',
            code: 'unavailable',
            message: '合成 Pi 依赖缺失',
            action: '配置专用 Pi',
          },
          {
            key: 'capture_permission',
            state: 'unknown',
            code: null,
            message: '合成录音权限未知',
            action: null,
          },
        ],
      }),
    });
    const view = render(<App bridge={fake.bridge} />);
    await screen.findByRole('button', { name: '打开首次设置' });
    expect(lifecycleWrites(fake)).toEqual([]);
    await userEvent.click(screen.getByRole('button', { name: '打开首次设置' }));
    await userEvent.click(await screen.findByRole('button', { name: '初始化本机资料库' }));
    const dialog = screen.getByRole('dialog', { name: '确认初始化本机资料库' });
    expect(lifecycleWrites(fake)).toEqual([]);
    await userEvent.click(within(dialog).getByRole('button', { name: '确认初始化' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(lifecycleWrites(fake).map((call) => call.command)).toEqual(['profile_initialize']);
    expect(fake.calls.find((call) => call.command === 'profile_initialize')?.payload).toEqual({});
    expect(fake.calls.some((call) => call.command === 'config_get')).toBe(true);
    await userEvent.click(screen.getByRole('button', { name: '启动后台服务' }));
    const start = await screen.findByRole('dialog', { name: '启动后台服务' });
    expect(lifecycleWrites(fake)).toHaveLength(1);
    await userEvent.click(within(start).getByRole('button', { name: '确认启动' }));
    await waitFor(() => expect(lifecycleWrites(fake)).toHaveLength(2));
    expect(lifecycleWrites(fake)[1]?.payload).toEqual({
      planToken: sample('service_plan').planToken,
    });
    expect(await screen.findByText('合成 Pi 依赖缺失')).toBeVisible();
    expect(screen.getByText('合成录音权限未知')).toBeVisible();
    expect(
      fake.calls.some((call) =>
        ['record_start', 'record_set_consent', 'record_set_auto', 'config_save'].includes(
          call.command,
        ),
      ),
    ).toBe(false);
    view.unmount();
    expect(lifecycleWrites(fake)).toHaveLength(2);
  });
  it('labels uninitialized default configuration and prevents editing or saving it', async () => {
    const fake = fixtureBridge({
      service_plan: ({ action }) =>
        plan(action, {
          state: 'uninitialized',
          canInitialize: true,
          canApply: false,
          planToken: null,
          expiresAtMs: null,
          reasons: ['profile_not_initialized'],
        }),
    });
    render(<App bridge={fake.bridge} />);
    await userEvent.click(screen.getByRole('link', { name: '设置' }));
    expect(await screen.findByLabelText('Pi 模型')).toBeDisabled();
    expect(screen.getByText(/下方仅展示未保存的默认值/)).toBeVisible();
    expect(fake.calls.some((call) => call.command === 'config_save')).toBe(false);
    expect(lifecycleWrites(fake)).toEqual([]);
  });
  it('retains an explicit initialization failure without claiming success or starting a service', async () => {
    const fake = fixtureBridge({
      service_plan: ({ action }) =>
        plan(action, {
          state: 'uninitialized',
          canInitialize: true,
          canApply: false,
          planToken: null,
          expiresAtMs: null,
          reasons: ['profile_not_initialized'],
        }),
      profile_initialize: () => {
        throw conflict();
      },
    });
    render(<App bridge={fake.bridge} />);
    await openServices();
    await userEvent.click(screen.getByRole('button', { name: '初始化本机资料库' }));
    await userEvent.click(screen.getByRole('button', { name: '确认初始化' }));
    await waitFor(() => expect(lifecycleWrites(fake)).toHaveLength(1));
    expect(
      await within(screen.getByRole('dialog')).findByText(/配置已被其他进程修改/),
    ).toBeVisible();
    expect(screen.queryByText(/本机资料库已初始化/)).not.toBeInTheDocument();
    expect(fake.calls.some((call) => call.command === 'service_apply')).toBe(false);
  });
  it.each([
    ['stop', '停止后台服务', '确认停止', 'running', 'stopped'],
    ['switch', '安全切换服务版本', '确认安全切换', 'incompatible', 'running'],
  ] as const)(
    'requires an action-specific plan and explicit %s confirmation',
    async (action, button, confirm, state, resultState) => {
      const fake = fixtureBridge({
        service_plan: ({ action: requested }) => plan(requested, { state }),
        service_apply: () => ({ ...sample('service_apply'), action, state: resultState }),
        config_get: () => ({ ...sample('config_get'), initialized: true }),
      });
      const view = render(<App bridge={fake.bridge} />);
      await openServices();
      await userEvent.click(screen.getByRole('button', { name: button }));
      const modal = await screen.findByRole('dialog');
      expect(
        fake.calls.some(
          (call) =>
            call.command === 'service_plan' &&
            (call.payload as { action: ServiceAction }).action === action,
        ),
      ).toBe(true);
      expect(lifecycleWrites(fake)).toEqual([]);
      await userEvent.click(within(modal).getByRole('button', { name: confirm }));
      await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
      expect(lifecycleWrites(fake)).toHaveLength(1);
      expect(lifecycleWrites(fake)[0]?.payload).toEqual({
        planToken: sample('service_plan').planToken,
      });
      expect(fake.calls.some((call) => call.command === 'config_get')).toBe(true);
      view.unmount();
      expect(lifecycleWrites(fake)).toHaveLength(1);
    },
  );
  it.each([
    ['busy', 'service_busy', '有活动录音'],
    ['unknown', 'unowned_service', '当前服务归属不明'],
    ['incompatible', 'incompatible_service', '当前服务版本不兼容'],
  ] as const)('explains %s and refuses to apply a blocked plan', async (state, reason, text) => {
    const fake = fixtureBridge({
      service_plan: ({ action }) =>
        plan(action, {
          state,
          canApply: false,
          planToken: null,
          expiresAtMs: null,
          reasons: [reason],
        }),
    });
    render(<App bridge={fake.bridge} />);
    await openServices();
    await userEvent.click(screen.getByRole('button', { name: '安全切换服务版本' }));
    const modal = await screen.findByRole('dialog');
    expect(within(modal).getByText(new RegExp(text))).toBeVisible();
    expect(within(modal).getByRole('button', { name: '确认安全切换' })).toBeDisabled();
    expect(lifecycleWrites(fake)).toEqual([]);
  });
  it('refuses initialization over existing or unknown profile contents', async () => {
    const fake = fixtureBridge({
      service_plan: ({ action }) =>
        plan(action, {
          state: 'uninitialized',
          canInitialize: false,
          canApply: false,
          planToken: null,
          expiresAtMs: null,
          reasons: ['profile_not_empty'],
        }),
    });
    render(<App bridge={fake.bridge} />);
    const region = await openServices();
    expect(await within(region).findByText(/资料目录已有内容/)).toBeVisible();
    expect(screen.queryByRole('button', { name: '初始化本机资料库' })).not.toBeInTheDocument();
    expect(lifecycleWrites(fake)).toEqual([]);
  });
  it('rejects a response containing a different action rather than confirming it', async () => {
    const fake = fixtureBridge({ service_plan: () => plan('start') });
    render(<App bridge={fake.bridge} />);
    await openServices();
    await userEvent.click(screen.getByRole('button', { name: '停止后台服务' }));
    expect(await screen.findByText(/服务返回的计划动作不匹配/)).toBeVisible();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(lifecycleWrites(fake)).toEqual([]);
  });
  it('never applies expired plans and does not display their opaque tokens', async () => {
    const fake = fixtureBridge({
      service_plan: ({ action }) => plan(action, { expiresAtMs: Date.now() - 1 }),
    });
    render(<App bridge={fake.bridge} />);
    await openServices();
    await userEvent.click(screen.getByRole('button', { name: '启动后台服务' }));
    const modal = await screen.findByRole('dialog');
    expect(within(modal).getByRole('button', { name: '确认启动' })).toBeDisabled();
    expect(document.body.textContent).not.toContain(sample('service_plan').planToken);
    expect(lifecycleWrites(fake)).toEqual([]);
  });
  it('consumes a failed plan and requires a fresh explicit plan after native state revalidation rejects it', async () => {
    let attempts = 0;
    let sequence = 0;
    const fake = fixtureBridge({
      service_plan: ({ action }) =>
        plan(action, {
          planToken: `fixture_lifecycle_token_${String(++sequence).padStart(16, '0')}`,
        }),
      config_get: () => ({ ...sample('config_get'), initialized: true }),
      service_apply: () => {
        if (++attempts === 1) throw conflict();
        return { ...sample('service_apply'), action: 'start', state: 'running' };
      },
    });
    render(<App bridge={fake.bridge} />);
    await openServices();
    await userEvent.click(screen.getByRole('button', { name: '启动后台服务' }));
    const modal = await screen.findByRole('dialog');
    await userEvent.click(within(modal).getByRole('button', { name: '确认启动' }));
    await waitFor(() =>
      expect(within(modal).getByRole('button', { name: '确认启动' })).toBeDisabled(),
    );
    expect(await within(modal).findByText(/配置已被其他进程修改/)).toBeVisible();
    expect(lifecycleWrites(fake)).toHaveLength(1);
    await userEvent.click(within(modal).getByRole('button', { name: '重新检查计划' }));
    await waitFor(() =>
      expect(within(modal).getByRole('button', { name: '确认启动' })).toBeEnabled(),
    );
    await userEvent.click(within(modal).getByRole('button', { name: '确认启动' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(lifecycleWrites(fake)).toHaveLength(2);
    expect(lifecycleWrites(fake)[1]?.payload).not.toEqual(lifecycleWrites(fake)[0]?.payload);
  });
  it('invalidates the previous token when a fresh plan check fails', async () => {
    let checks = 0;
    const fake = fixtureBridge({
      service_plan: ({ action }) => {
        if (action === 'stop' && ++checks > 1) throw conflict();
        return plan(action);
      },
    });
    render(<App bridge={fake.bridge} />);
    await openServices();
    await userEvent.click(screen.getByRole('button', { name: '停止后台服务' }));
    const modal = await screen.findByRole('dialog');
    expect(within(modal).getByRole('button', { name: '确认停止' })).toBeEnabled();
    await userEvent.click(within(modal).getByRole('button', { name: '重新检查计划' }));
    expect(await within(modal).findByText(/配置已被其他进程修改/)).toBeVisible();
    expect(within(modal).getByRole('button', { name: '确认停止' })).toBeDisabled();
    expect(lifecycleWrites(fake)).toEqual([]);
  });
  it('honors configuration dirty-state navigation before exposing lifecycle operations', async () => {
    const fake = fixtureBridge({
      config_get: () => ({ ...sample('config_get'), initialized: true }),
    });
    render(<App bridge={fake.bridge} />);
    await userEvent.click(screen.getByRole('link', { name: '设置' }));
    await userEvent.type(await screen.findByLabelText('Pi 模型'), 'unsaved');
    await userEvent.click(screen.getByRole('button', { name: '打开服务设置' }));
    expect(screen.getByRole('dialog', { name: '设置尚未保存' })).toBeVisible();
    expect(screen.queryByRole('region', { name: '首次设置与后台服务' })).not.toBeInTheDocument();
    expect(lifecycleWrites(fake)).toEqual([]);
    await userEvent.click(screen.getByRole('button', { name: '留在设置' }));
    expect(screen.getByLabelText('Pi 模型')).toHaveValue(
      `${sample('config_get').values.piModel}unsaved`,
    );
  });
});
