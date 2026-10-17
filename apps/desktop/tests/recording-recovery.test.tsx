import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../src/App';
import {
  conflict,
  fixtureBridge,
  meetingFixture,
  NativeFailure,
  sample,
  taskFixture,
} from './fixtures';
describe('Explicit recording and recovery controls (synthetic transport only)', () => {
  it('confirms participant consent for auto mode, reports actual state and seals on explicit stop', async () => {
    let status = {
      ...sample('record_status'),
      state: 'recording' as const,
      meetingId: meetingFixture().meetingId,
      startedAtMs: 1,
    };
    const fake = fixtureBridge({
      record_status: () => status,
      record_auto_set: (p) => {
        status = { ...status, autoRecord: p.enabled };
        return status;
      },
      record_stop: () => ({ ...sample('record_status'), state: 'sealing' }),
    });
    render(<App bridge={fake.bridge} />);
    await screen.findByRole('button', { name: '停止并封存' });
    expect(fake.calls.some((c) => c.command === 'record_stop')).toBe(false);
    await userEvent.click(screen.getByRole('button', { name: '启用自动录音' }));
    expect(fake.calls.some((c) => c.command === 'record_auto_set')).toBe(false);
    await userEvent.click(screen.getByRole('button', { name: '确认同意并启用' }));
    await screen.findByRole('button', { name: '关闭自动录音' });
    expect(fake.calls.find((c) => c.command === 'record_auto_set')?.payload).toMatchObject({
      enabled: true,
      confirmRecordingConsent: true,
    });
    await userEvent.click(screen.getByRole('button', { name: '关闭自动录音' }));
    await screen.findByRole('button', { name: '启用自动录音' });
    expect(fake.calls.filter((c) => c.command === 'record_auto_set').at(-1)?.payload).toMatchObject(
      { enabled: false, confirmRecordingConsent: false },
    );
    await userEvent.click(screen.getByRole('button', { name: '停止并封存' }));
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.command === 'record_stop')).toHaveLength(1),
    );
    expect(fake.calls.some((c) => c.command === 'record_start')).toBe(false);
  });
  it('does not report success when auto state carries a native error', async () => {
    const fake = fixtureBridge({
      record_auto_set: () => ({
        ...sample('record_status'),
        error: {
          code: 'unavailable',
          reason: 'incompatible_service',
          message: '合成后台服务不兼容',
          retryable: false,
          fieldErrors: [],
        },
      }),
    });
    render(<App bridge={fake.bridge} />);
    await userEvent.click(await screen.findByRole('button', { name: '启用自动录音' }));
    await userEvent.click(screen.getByRole('button', { name: '确认同意并启用' }));
    await waitFor(() =>
      expect(screen.getAllByText(/合成后台服务不兼容/).length).toBeGreaterThan(0),
    );
    expect(screen.getByRole('dialog')).toBeVisible();
    expect(screen.queryByText('自动录音已启用')).not.toBeInTheDocument();
  });
  it('keeps edit draft on stale revision and reports busy trash rejection without hiding the dialog', async () => {
    const meeting = meetingFixture();
    const fake = fixtureBridge({
      meeting_list: () => ({ ...sample('meeting_list'), items: [meeting], total: 1 }),
      meeting_update: () => {
        throw conflict();
      },
      meeting_trash: () => {
        throw new NativeFailure({
          code: 'conflict',
          reason: 'busy',
          message: '合成会议仍有活动任务，保留原内容',
          retryable: false,
          fieldErrors: [],
        });
      },
    });
    render(<App bridge={fake.bridge} />);
    await userEvent.click(await screen.findByRole('button', { name: '编辑' }));
    await userEvent.type(screen.getByLabelText('备注'), '保留草稿');
    await userEvent.click(screen.getByRole('button', { name: '保存' }));
    await waitFor(() =>
      expect(within(screen.getByRole('dialog')).getByRole('alert')).toHaveTextContent('其他进程'),
    );
    expect(screen.getByLabelText('备注')).toHaveValue('仅供测试保留草稿');
    await userEvent.click(screen.getByRole('button', { name: '放弃' }));
    await userEvent.click(screen.getByRole('checkbox', { name: '选择 合成设计讨论' }));
    await userEvent.click(screen.getByRole('button', { name: '移入回收站' }));
    await userEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: '移入回收站' }),
    );
    await waitFor(() =>
      expect(within(screen.getByRole('dialog')).getByRole('alert')).toHaveTextContent('活动任务'),
    );
    expect(screen.getByRole('button', { name: '合成设计讨论' })).toBeVisible();
  });
  it('accepts 256 Unicode title characters rather than truncating surrogate pairs', async () => {
    const meeting = meetingFixture();
    const fake = fixtureBridge({
      meeting_list: () => ({ ...sample('meeting_list'), items: [meeting], total: 1 }),
      meeting_update: (p) => ({ ...meeting, metadata: p.metadata, revision: meeting.revision + 1 }),
    });
    render(<App bridge={fake.bridge} />);
    await userEvent.click(await screen.findByRole('button', { name: '编辑' }));
    const input = screen.getByLabelText('会议标题');
    fireEvent.change(input, { target: { value: '😀'.repeat(257) } });
    expect(input).toHaveValue('😀'.repeat(256));
    await userEvent.click(screen.getByRole('button', { name: '保存' }));
    await waitFor(() =>
      expect(fake.calls.find((c) => c.command === 'meeting_update')?.payload).toMatchObject({
        metadata: { title: '😀'.repeat(256) },
      }),
    );
  });
  it('requeues only on explicit click and reads task diagnostics through task_get', async () => {
    const task = taskFixture();
    const fake = fixtureBridge({
      task_list: () => ({ ...sample('task_list'), items: [task] }),
      task_get: () => ({ ...sample('task_get'), task, diagnostics: ['合成脱敏诊断'] }),
      processing_requeue: () => ({ ...task, status: 'queued' }),
    });
    render(<App bridge={fake.bridge} />);
    await userEvent.click(screen.getByRole('link', { name: '处理队列' }));
    await userEvent.click(await screen.findByRole('button', { name: '重新入队' }));
    await waitFor(() =>
      expect(fake.calls.some((c) => c.command === 'processing_requeue')).toBe(true),
    );
    await userEvent.click(screen.getByRole('button', { name: '第 1 次 · 转写' }));
    expect(await screen.findByText('合成脱敏诊断')).toBeVisible();
    expect(fake.calls.find((c) => c.command === 'task_get')?.payload).toEqual({
      taskId: task.taskId,
    });
  });
});
