import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../src/App';
import { conflict, fixtureBridge, meetingFixture, sample } from './fixtures';
import type { DesktopCommandMap } from '../src/contracts.generated';
type Page = DesktopCommandMap['meeting_list']['response'];
describe('Server pagination and stale read isolation', () => {
  it('resets a stale cursor with a visible explanation instead of mixing snapshots', async () => {
    const row = meetingFixture();
    const fake = fixtureBridge({
      meeting_list: (p) => {
        if (p.cursor) throw conflict();
        return {
          ...sample('meeting_list'),
          items: [row],
          total: 26,
          nextCursor: 'fixture_cursor_000002',
        };
      },
    });
    render(<App bridge={fake.bridge} />);
    await screen.findByRole('button', { name: row.metadata.title });
    await userEvent.click(screen.getByRole('button', { name: '下一页' }));
    expect(
      await screen.findByText('会议列表已变化，已返回第一页。请重新核对选择后继续。'),
    ).toBeVisible();
    expect(screen.getByText(/共 26 场 · 第 1 页/)).toBeVisible();
    expect(screen.getByRole('button', { name: '上一页' })).toBeDisabled();
    expect(screen.getByRole('button', { name: row.metadata.title })).toBeVisible();
  });
  it('never replaces newer search results when an earlier native read resolves late', async () => {
    let release: ((page: Page) => void) | undefined;
    const row = meetingFixture();
    const fake = fixtureBridge({
      meeting_list: (p) =>
        p.filter.query === 'old'
          ? new Promise<Page>((resolve) => {
              release = resolve;
            })
          : {
              ...sample('meeting_list'),
              items: [
                { ...row, metadata: { ...row.metadata, title: p.filter.query || '初始合成会议' } },
              ],
              total: 1,
            },
    });
    render(<App bridge={fake.bridge} />);
    await screen.findByRole('button', { name: '初始合成会议' });
    const input = screen.getByRole('textbox', { name: '搜索会议' });
    await userEvent.type(input, 'old');
    await waitFor(() => expect(release).toBeDefined());
    await userEvent.clear(input);
    await userEvent.type(input, 'new');
    await screen.findByRole('button', { name: 'new' });
    await act(async () => {
      release?.({
        ...sample('meeting_list'),
        items: [{ ...row, metadata: { ...row.metadata, title: '不应显示的旧结果' } }],
        total: 1,
      });
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'new' })).toBeVisible();
    expect(screen.queryByRole('button', { name: '不应显示的旧结果' })).not.toBeInTheDocument();
  });
});
