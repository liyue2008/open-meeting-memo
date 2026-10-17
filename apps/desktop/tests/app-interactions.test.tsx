import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../src/App';
import {
  fixtureBridge,
  meetingFixture,
  sample,
  conflict,
  taskFixture,
  selectionFixture,
} from './fixtures';
import { systemDark } from './setup';
const user = () => userEvent.setup();
async function visit(name: string) {
  await user().click(screen.getByRole('link', { name }));
}
describe('App real typed bridge interactions with synthetic transport', () => {
  it('defaults seven local days and sends search/filter/sort/page requests to native', async () => {
    const meeting = meetingFixture();
    const fake = fixtureBridge({
      meeting_list: (p) => ({
        ...sample('meeting_list'),
        items: p.cursor ? [] : [meeting],
        nextCursor: p.cursor ? null : 'fixture_cursor_page_000002',
        total: 26,
        search: { complete: true, indexed: 26, pending: 0 },
      }),
    });
    render(<App bridge={fake.bridge} />);
    await screen.findByRole('button', { name: meeting.metadata.title });
    const first = fake.calls.find((c) => c.command === 'meeting_list')!.payload as {
      filter: { date: unknown };
      pageSize: number;
    };
    expect(first.pageSize).toBe(25);
    expect(first.filter.date).not.toBeNull();
    await user().click(screen.getByRole('button', { name: '下一页' }));
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.command === 'meeting_list').at(-1)?.payload).toMatchObject({
        cursor: 'fixture_cursor_page_000002',
        expectedRevision: sample('meeting_list').revision,
      }),
    );
    expect(await screen.findByText('没有符合条件的会议')).toBeVisible();
    await user().type(screen.getByRole('textbox', { name: '搜索会议' }), '合成');
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.command === 'meeting_list').at(-1)?.payload).toMatchObject({
        filter: { query: '合成' },
        cursor: null,
      }),
    );
    await user().selectOptions(screen.getByLabelText('日期'), 'week');
    await user().selectOptions(screen.getByLabelText('来源', { exact: true }), 'imported');
    await user().selectOptions(screen.getByLabelText('排序'), 'duration_desc');
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.command === 'meeting_list').at(-1)?.payload).toMatchObject({
        filter: { sources: ['imported'], sort: 'duration_desc' },
      }),
    );
  });
  it('preserves library filters and the search draft when returning from a detail', async () => {
    const meeting = meetingFixture();
    const fake = fixtureBridge({
      meeting_list: () => ({ ...sample('meeting_list'), items: [meeting], total: 1 }),
      meeting_get: () => ({ ...sample('meeting_get'), meeting }),
    });
    render(<App bridge={fake.bridge} />);
    await screen.findByRole('button', { name: meeting.metadata.title });
    await user().type(screen.getByRole('textbox', { name: '搜索会议' }), '保留搜索');
    await user().selectOptions(screen.getByLabelText('日期'), 'all');
    await user().click(screen.getByRole('button', { name: meeting.metadata.title }));
    await screen.findByRole('heading', { name: meeting.metadata.title });
    expect(screen.getByRole('main')).toHaveFocus();
    await user().click(screen.getByRole('button', { name: '← 返回会议库' }));
    expect(screen.getByRole('textbox', { name: '搜索会议' })).toHaveValue('保留搜索');
    expect(screen.getByLabelText('日期')).toHaveValue('all');
  });
  it('edits metadata with expected revision and retains recordings on trash/restore', async () => {
    let meeting = meetingFixture();
    const fake = fixtureBridge({
      meeting_list: () => ({ ...sample('meeting_list'), items: [meeting], total: 1 }),
      meeting_update: (p) => {
        expect(p.expectedRevision).toBe(meeting.revision);
        meeting = { ...meeting, metadata: p.metadata, revision: meeting.revision + 1 };
        return meeting;
      },
      meeting_trash: (p) => {
        expect(p.meetings).toEqual([
          { meetingId: meeting.meetingId, expectedRevision: meeting.revision },
        ]);
        meeting = {
          ...meeting,
          trashed: true,
          revision: meeting.revision + 1,
          capabilities: { ...meeting.capabilities, trash: false, restore: true },
        };
        return { meetings: [meeting] };
      },
      meeting_restore: () => {
        meeting = { ...meeting, trashed: false };
        return { meetings: [meeting] };
      },
    });
    render(<App bridge={fake.bridge} />);
    await screen.findByRole('button', { name: '合成设计讨论' });
    await user().click(screen.getByRole('button', { name: '编辑' }));
    const dialog = screen.getByRole('dialog');
    await user().clear(within(dialog).getByLabelText('会议标题'));
    await user().type(within(dialog).getByLabelText('会议标题'), '更新后的合成标题');
    await user().click(within(dialog).getByRole('button', { name: '保存' }));
    await screen.findByRole('button', { name: '更新后的合成标题' });
    await user().click(screen.getByRole('checkbox', { name: '选择 更新后的合成标题' }));
    await user().click(screen.getByRole('button', { name: '移入回收站' }));
    await user().click(
      within(screen.getByRole('dialog')).getByRole('button', { name: '移入回收站' }),
    );
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    await user().click(screen.getByRole('checkbox', { name: '选择 更新后的合成标题' }));
    await user().click(screen.getByRole('button', { name: '恢复' }));
    await user().click(screen.getByRole('button', { name: '确认恢复' }));
    await waitFor(() => expect(fake.calls.some((c) => c.command === 'meeting_restore')).toBe(true));
    expect(fake.calls.some((c) => (c.command as string).includes('purge'))).toBe(false);
  });
  it('native import is explicit and cancellation releases its handle without recording', async () => {
    const fake = fixtureBridge({ import_select: () => ({ selection: selectionFixture() }) });
    render(<App bridge={fake.bridge} />);
    await screen.findByText('没有符合条件的会议');
    await user().click(screen.getByRole('button', { name: '＋ 新建会议' }));
    await user().click(screen.getByRole('button', { name: '选择音频文件' }));
    await screen.findByText(/声道/);
    expect(fake.calls.some((c) => c.command === 'record_start')).toBe(false);
    await user().click(screen.getByRole('button', { name: '关闭对话框' }));
    await waitFor(() => expect(fake.calls.some((c) => c.command === 'import_cancel')).toBe(true));
  });
  it('commits selected native import with explicit timestamp/title and no arbitrary path', async () => {
    const fake = fixtureBridge({ import_select: () => ({ selection: selectionFixture() }) });
    render(<App bridge={fake.bridge} />);
    await screen.findByText('没有符合条件的会议');
    await user().click(screen.getByRole('button', { name: '＋ 新建会议' }));
    await user().type(screen.getByLabelText('会议标题'), '合成导入');
    await user().click(screen.getByRole('button', { name: '选择音频文件' }));
    await screen.findByText(/声道/);
    fireEvent.change(screen.getByLabelText('原会议开始时间（本地）'), {
      target: { value: '2026-10-17T15:00' },
    });
    await user().click(screen.getByRole('button', { name: '导入' }));
    await waitFor(() => expect(fake.calls.some((c) => c.command === 'import_commit')).toBe(true));
    const payload = fake.calls.find((c) => c.command === 'import_commit')!.payload;
    expect(payload).toMatchObject({
      selectionHandle: selectionFixture().selectionHandle,
      metadata: { title: '合成导入' },
    });
    expect(payload).not.toHaveProperty('path');
  });
  it('never starts recording implicitly; consent and explicit click are required', async () => {
    const fake = fixtureBridge();
    render(<App bridge={fake.bridge} />);
    await screen.findByText('没有符合条件的会议');
    expect(fake.calls.some((c) => c.command === 'record_start')).toBe(false);
    await user().click(screen.getByRole('button', { name: '＋ 新建会议' }));
    await user().type(screen.getByLabelText('会议标题'), '合成手动会议');
    expect(screen.getByRole('button', { name: '开始录音' })).toBeDisabled();
    await user().click(screen.getByRole('checkbox', { name: /我确认参与者同意/ }));
    await user().click(screen.getByRole('button', { name: '开始录音' }));
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.command === 'record_start')).toHaveLength(1),
    );
    expect(fake.calls.find((c) => c.command === 'record_start')?.payload).toMatchObject({
      title: '合成手动会议',
      confirmRecordingConsent: true,
    });
  });
  it('retains settings draft on conflict, blocks navigation and discards only explicitly', async () => {
    const fake = fixtureBridge({
      config_validate: () => ({ ...sample('config_validate'), valid: true }),
      config_save: () => {
        throw conflict();
      },
    });
    render(<App bridge={fake.bridge} />);
    await visit('设置');
    const model = await screen.findByLabelText('Pi 模型');
    await user().clear(model);
    await user().type(model, 'synthetic-model');
    await waitFor(() => expect(screen.getByRole('button', { name: '保存配置' })).toBeEnabled());
    await user().click(screen.getByRole('button', { name: '保存配置' }));
    await waitFor(() =>
      expect(screen.getAllByText(/配置已被其他进程修改/).length).toBeGreaterThan(0),
    );
    expect(model).toHaveValue('synthetic-model');
    await visit('会议库');
    expect(screen.getByRole('dialog')).toHaveAccessibleName('设置尚未保存');
    await user().click(screen.getByRole('button', { name: '留在设置' }));
    expect(model).toHaveValue('synthetic-model');
    await visit('会议库');
    await user().click(screen.getByRole('button', { name: '放弃并离开' }));
    await screen.findByRole('heading', { name: '会议库' });
    expect(fake.calls.find((c) => c.command === 'config_save')?.payload).toMatchObject({
      expectedRevision: null,
      patch: { piModel: 'synthetic-model' },
    });
  });
  it('saves settings atomically before leaving without credentials or auto-record patches', async () => {
    const fake = fixtureBridge({
      config_validate: () => ({ ...sample('config_validate'), valid: true }),
      config_save: (p) => ({
        ...sample('config_get'),
        revision: 'b'.repeat(64),
        values: { ...sample('config_get').values, ...p.patch },
      }),
    });
    render(<App bridge={fake.bridge} />);
    await visit('设置');
    const threads = await screen.findByLabelText('转写线程数');
    await user().clear(threads);
    await user().type(threads, '2');
    await visit('使用帮助');
    await user().click(screen.getByRole('button', { name: '保存并离开' }));
    await screen.findByRole('heading', { name: '使用帮助' });
    const payload = fake.calls.find((c) => c.command === 'config_save')!.payload as {
      patch: unknown;
    };
    expect(payload.patch).toEqual({ asrThreads: 2 });
  });
  it('follows live OS theme until overridden, persists layout only and keeps business mounted', async () => {
    const fake = fixtureBridge();
    render(<App bridge={fake.bridge} />);
    await screen.findByText('没有符合条件的会议');
    expect(document.documentElement).toHaveAttribute('data-theme', 'light');
    act(() => systemDark(true));
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark');
    await user().type(screen.getByRole('textbox', { name: '搜索会议' }), 'keep-draft');
    await user().selectOptions(screen.getByLabelText('外观主题'), 'light');
    act(() => systemDark(false));
    act(() => systemDark(true));
    expect(document.documentElement).toHaveAttribute('data-theme', 'light');
    expect(screen.getByRole('textbox', { name: '搜索会议' })).toHaveValue('keep-draft');
    await user().click(screen.getByRole('button', { name: '收起侧栏' }));
    expect(document.documentElement).toHaveAttribute('data-sidebar', 'collapsed');
    fireEvent.focus(screen.getByRole('link', { name: '处理队列' }));
    expect(screen.getByRole('tooltip')).toHaveTextContent('处理队列');
    fireEvent.keyDown(screen.getByRole('link', { name: '处理队列' }), { key: 'Escape' });
    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();
    expect(Object.keys(localStorage).sort()).toEqual(['omm.sidebar', 'omm.theme']);
    expect(localStorage.getItem('omm.sidebar')).toBe('collapsed');
  });
  it('shows real queue retry/cancel intent and retained diagnostic limitations', async () => {
    const task = taskFixture();
    const fake = fixtureBridge({
      task_list: () => ({ ...sample('task_list'), items: [task] }),
      processing_cancel: () => ({ ...task, cancelRequested: true }),
    });
    render(<App bridge={fake.bridge} />);
    await visit('处理队列');
    await screen.findByRole('button', { name: '重试失败阶段' });
    await user().click(screen.getByRole('button', { name: '重试失败阶段' }));
    await waitFor(() =>
      expect(fake.calls.some((c) => c.command === 'processing_retry')).toBe(true),
    );
    await user().click(screen.getByRole('button', { name: '取消任务' }));
    await user().click(screen.getByRole('button', { name: '确认取消' }));
    await waitFor(() =>
      expect(fake.calls.some((c) => c.command === 'processing_cancel')).toBe(true),
    );
    await visit('状态与诊断');
    await screen.findByRole('heading', { name: '通知调用记录' });
    expect(screen.getByText(/调用成功只表示系统脚本返回/)).toBeVisible();
    expect(fake.calls.some((c) => c.command === 'diagnostics_get')).toBe(true);
  });
});
