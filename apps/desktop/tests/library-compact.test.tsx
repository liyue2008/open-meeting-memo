import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../src/App';
import { render } from './render';
import { fixtureBridge, meetingFixture, sample } from './fixtures';

describe('features/library compact default controls', () => {
  it('labels search visibly and keeps advanced filters and cloud guidance explicitly reachable', async () => {
    const fake = fixtureBridge({
      meeting_list: () => ({ ...sample('meeting_list'), items: [meetingFixture()] }),
    });
    render(<App bridge={fake.bridge} />);
    await screen.findByRole('button', { name: '合成设计讨论' });
    expect(screen.getByText('搜索会议', { selector: 'label' })).toBeVisible();
    expect(screen.getByLabelText('日期')).toHaveValue('7');
    const advanced = screen.getByRole('button', { name: /高级筛选与数据说明/ });
    expect(advanced).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(advanced);
    await waitFor(() => expect(screen.getByLabelText('来源', { exact: true })).toBeVisible());
    expect(screen.getByText(/云端模型会收到逐字稿文本/)).toBeVisible();
    await userEvent.selectOptions(screen.getByLabelText('来源', { exact: true }), 'imported');
    await userEvent.click(advanced);
    expect(advanced).toHaveTextContent('已应用 1 项');
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.command === 'meeting_list').at(-1)?.payload).toMatchObject({
        filter: { sources: ['imported'] },
      }),
    );
  });
});
