import { act, render, screen } from '@testing-library/react';
import { focusManager, QueryClient } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../src/App';
import { createBridge } from '../src/bridge/client';
import type { RecordStatus } from '../src/contracts.generated';
import { fixtureBridge, sample } from './fixtures';

const tick = async (ms: number) => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
};
afterEach(() => {
  vi.useRealTimers();
  focusManager.setFocused(undefined);
});

describe('RecordingBar observes synthetic automatic state changes without issuing recording commands', () => {
  it('polls idle → recording → sealing → idle at the existing interval', async () => {
    vi.useFakeTimers();
    let status: RecordStatus = sample('record_status');
    const fake = fixtureBridge({ record_status: () => status });
    render(<App bridge={fake.bridge} />);
    await tick(50);
    expect(screen.getByText('当前未录音')).toBeVisible();
    for (const [state, label] of [
      ['recording', /正在录音 ·/],
      ['sealing', '正在封存，请等待'],
      ['idle', '当前未录音'],
    ] as const) {
      status = { ...status, state };
      await tick(10000);
      expect(screen.getByText(label)).toBeVisible();
    }
    expect(
      fake.calls.some((c) =>
        ['record_start', 'record_stop', 'record_auto_set'].includes(c.command),
      ),
    ).toBe(false);
  });
  it('refreshes an automatic change made while unfocused on focus restoration', async () => {
    vi.useFakeTimers();
    let status: RecordStatus = sample('record_status');
    const fake = fixtureBridge({ record_status: () => status });
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: 1000 } },
    });
    render(<App bridge={fake.bridge} client={client} />);
    await tick(50);
    act(() => focusManager.setFocused(false));
    status = { ...status, state: 'recording' };
    await tick(2000);
    expect(screen.getByText('当前未录音')).toBeVisible();
    act(() => focusManager.setFocused(true));
    await tick(50);
    expect(screen.getByText(/正在录音 ·/)).toBeVisible();
    client.clear();
  });
  it('reports transport failure after a successful idle read as unknown, not idle', async () => {
    vi.useFakeTimers();
    let fail = false;
    const fake = fixtureBridge({
      record_status: () => {
        if (fail) throw new Error('synthetic status unavailable');
        return sample('record_status');
      },
    });
    render(<App bridge={fake.bridge} />);
    await tick(50);
    fail = true;
    await tick(10000);
    expect(screen.getByText('录音状态与已保存的自动录音偏好未知，不能更改。')).toBeVisible();
    expect(screen.queryByText('当前未录音')).not.toBeInTheDocument();
  });
  it('keeps an explicit unknown state and unknown saved preferences unknown', async () => {
    vi.useFakeTimers();
    const fake = fixtureBridge({
      record_status: () => ({
        ...sample('record_status'),
        state: 'unknown',
        autoRecord: null,
        recordingConsent: null,
      }),
    });
    render(<App bridge={fake.bridge} />);
    await tick(50);
    expect(screen.getByText('录音状态未知')).toBeVisible();
    expect(screen.getByRole('button', { name: '自动录音偏好未知' })).toBeDisabled();
    expect(screen.queryByText('当前未录音')).not.toBeInTheDocument();
  });
  it('strictly rejects a missing state field rather than interpreting it as idle', async () => {
    vi.useFakeTimers();
    const { state: omitted, ...missing } = sample('record_status');
    expect(omitted).toBe('idle');
    const bridge = createBridge(async (command, { request }) => ({
      schemaVersion: 1,
      requestId: request.requestId,
      data: command === 'record_status' ? missing : sample(command),
    }));
    render(<App bridge={bridge} />);
    await tick(50);
    expect(screen.getByText('录音状态与已保存的自动录音偏好未知，不能更改。')).toBeVisible();
    expect(screen.queryByText('当前未录音')).not.toBeInTheDocument();
  });
});
