import { act, render, renderHook, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useAppearance } from '../src/app/preferences';
import { SafeMarkdown, readOnlyActions, timestampMs } from '../src/features/artifacts';
import { configDraft, configPatch } from '../src/features/settings';
import { sample } from './fixtures';
import { systemDark } from './setup';
describe('Appearance persistence and safe degradation', () => {
  it('restores only layout preferences across remount and tracks system live', () => {
    localStorage.setItem('omm.theme', 'dark');
    localStorage.setItem('omm.sidebar', 'collapsed');
    const first = renderHook(useAppearance);
    expect(first.result.current.theme).toBe('dark');
    expect(first.result.current.collapsed).toBe(true);
    first.unmount();
    const next = renderHook(useAppearance);
    act(() => next.result.current.setTheme('system'));
    act(() => systemDark(false));
    expect(document.documentElement.dataset.theme).toBe('light');
    act(() => systemDark(true));
    expect(document.documentElement.dataset.theme).toBe('dark');
    next.unmount();
  });
  it('reports malformed, unreadable and unwritable storage without disabling the UI', () => {
    localStorage.setItem('omm.theme', 'untrusted');
    localStorage.setItem('omm.sidebar', 'untrusted');
    const invalid = renderHook(useAppearance);
    expect(invalid.result.current.theme).toBe('system');
    expect(invalid.result.current.collapsed).toBe(false);
    expect(invalid.result.current.warning).toContain('损坏');
    invalid.unmount();
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const denied = renderHook(useAppearance);
    expect(denied.result.current.warning).toContain('无法读取');
    get.mockRestore();
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    act(() => denied.result.current.setTheme('light'));
    expect(denied.result.current.theme).toBe('light');
    expect(denied.result.current.warning).toContain('无法保存');
  });
});
describe('Meeting Markdown contains no execution, network media or mutable actions', () => {
  it('renders only bounded local timestamps and inert text from hostile Markdown', () => {
    const locate = vi.fn();
    const { container } = render(
      <SafeMarkdown
        duration={40000}
        locate={locate}
        text={
          '# 合成\n<script>window.stolen=true</script>\n<iframe src="https://untrusted.invalid"/>\n<img src="https://untrusted.invalid" onerror="steal()">\n\n![remote](https://untrusted.invalid/pixel)\n[link](https://untrusted.invalid) [script](javascript:alert(1))\n- [x] 只读行动\n[00:35] [00:99] [01:00] [seek](#t=9999999999999999999999)\n```html\n<script>inert()</script>\n```'
        }
      />,
    );
    expect(container.querySelectorAll('script,iframe,img,a,input,object,embed')).toHaveLength(0);
    expect(screen.getAllByRole('button')).toHaveLength(1);
    screen.getByRole('button', { name: '[00:35]' }).click();
    expect(locate).toHaveBeenCalledWith(35000);
  });
  it('extracts only recognizable read-only items and validates clock fields', () => {
    expect(
      readOnlyActions(
        '# 合成\n- 不推断\n## 行动项\n- [ ] 甲做事项\n1. 乙做事项\n## 结论\n- 不算行动',
      ),
    ).toEqual(['甲做事项', '乙做事项']);
    expect(readOnlyActions('没有结构化行动项')).toEqual([]);
    expect(timestampMs('01:02:03')).toBe(3723000);
    for (const text of ['60:00', '01:60:00', '00:99', '-1:00', 'NaN'])
      expect(timestampMs(text)).toBeNull();
  });
});
describe('Config draft conversion has a precise whitelist and no secret or auto-record writes', () => {
  it('rejects malformed integers and sends only actual normalized changes', () => {
    const base = sample('config_get').values;
    const draft = configDraft(base);
    expect(configPatch(base, draft)).toEqual({});
    for (const value of ['', '-1', '1.5', '2e4', 'Infinity', '9007199254740993'])
      expect(() => configPatch(base, { ...draft, asrThreads: value })).toThrow('安全整数');
    expect(
      configPatch(base, {
        ...draft,
        asrThreads: '2',
        piNodeExecutable: '',
        rtcLogDirs: ' /fixture/a \n\n/fixture/b',
      }),
    ).toMatchObject({ asrThreads: 2, rtcLogDirs: ['/fixture/a', '/fixture/b'] });
    expect(
      configPatch(
        { ...base, piNodeExecutable: '/fixture/node' },
        { ...draft, piNodeExecutable: '' },
      ),
    ).toHaveProperty('piNodeExecutable', null);
  });
});
