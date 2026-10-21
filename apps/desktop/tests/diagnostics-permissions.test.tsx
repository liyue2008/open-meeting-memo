import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it } from 'vitest';
import { App } from '../src/App';
import { fixtureBridge, conflict } from './fixtures';
import type { PermissionResult } from '../src/contracts.generated';

it('diagnostics permissions require a click, use one typed operation and do not record', async () => {
  const fake = fixtureBridge();
  render(<App bridge={fake.bridge} />);
  await userEvent.click(await screen.findByRole('link', { name: '状态与诊断' }));
  const button = await screen.findByRole('button', { name: '申请录音权限' });
  expect(fake.calls.some(c => c.command === 'permissions_request')).toBe(false);
  await userEvent.click(button);
  await waitFor(() => expect(fake.calls.some(c => c.command === 'permissions_request')).toBe(true));
  const request = fake.calls.find(c => c.command === 'permissions_request')!;
  expect(request.payload).toEqual({ action: 'request', operationId: expect.stringMatching(/^[a-f0-9-]{36}$/) });
  expect(fake.calls.some(c => c.command === 'record_start')).toBe(false);
  expect(await screen.findByText(/系统音频：未知/)).toBeInTheDocument();
});

it('pending permission cancellation uses the same typed command/operation and never claims OS cancellation', async () => {
  let finish: ((value: PermissionResult) => void) | undefined;
  const fake = fixtureBridge({ permissions_request: ({ action }) => {
    if (action === 'request') return new Promise<PermissionResult>(resolve => { finish = resolve; });
    const value: PermissionResult = { microphone: 'unknown', systemAudio: 'unknown', outcome: 'cancelled' };
    finish!(value); return value;
  } });
  render(<App bridge={fake.bridge} />);
  await userEvent.click(await screen.findByRole('link', { name: '状态与诊断' }));
  await userEvent.click(await screen.findByRole('button', { name: '申请录音权限' }));
  expect(screen.getByRole('button', { name: '申请录音权限' })).toBeDisabled();
  await userEvent.click(await screen.findByRole('button', { name: '取消等待' }));
  await screen.findByText('等待已取消');
  const calls = fake.calls.filter(c => c.command === 'permissions_request');
  expect(calls).toHaveLength(2);
  expect(calls[1]!.payload).toEqual({ ...(calls[0]!.payload as object), action: 'cancel' });
  expect(screen.getByText(/取消或超时不能关闭系统同意框/)).toBeInTheDocument();
});

it.each(['timed_out', 'cleanup_failed'] as const)('shows real %s without success or recording', async outcome => {
  const fake = fixtureBridge({ permissions_request: () => ({ microphone: 'unknown', systemAudio: 'unknown', outcome }) });
  render(<App bridge={fake.bridge} />);
  await userEvent.click(await screen.findByRole('link', { name: '状态与诊断' }));
  await userEvent.click(await screen.findByRole('button', { name: '申请录音权限' }));
  await screen.findByText(outcome === 'timed_out' ? '等待已超时' : /权限资源清理失败/);
  expect(fake.calls.some(c => c.command === 'record_start')).toBe(false);
});

it('busy error remains an error and does not retry or start recording', async () => {
  const fake = fixtureBridge({ permissions_request: () => { throw conflict(); } });
  render(<App bridge={fake.bridge} />);
  await userEvent.click(await screen.findByRole('link', { name: '状态与诊断' }));
  await userEvent.click(await screen.findByRole('button', { name: '申请录音权限' }));
  await screen.findByText(/配置已被其他进程修改/);
  expect(fake.calls.filter(c => c.command === 'permissions_request')).toHaveLength(1);
});
