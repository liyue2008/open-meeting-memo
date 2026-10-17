import type { DateRange } from '../contracts.generated';
export type DatePreset = 'today' | 'week' | '3' | '7' | 'all' | 'custom';
export const localDay = (d: Date): string =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
export function dateRange(
  preset: DatePreset,
  now = new Date(),
  from = '',
  to = '',
): DateRange | null {
  if (preset === 'all') return null;
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const end = new Date(start);
  end.setDate(end.getDate() + 1);
  if (preset === 'week') start.setDate(start.getDate() - ((start.getDay() + 6) % 7));
  if (preset === '3' || preset === '7') start.setDate(start.getDate() - (Number(preset) - 1));
  if (preset === 'custom') {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(from) || !/^\d{4}-\d{2}-\d{2}$/.test(to))
      throw new Error('请选择完整日期。');
    const [y, m, d] = from.split('-').map(Number);
    const [ey, em, ed] = to.split('-').map(Number);
    start.setFullYear(y!, m! - 1, d!);
    end.setFullYear(ey!, em! - 1, ed!);
    end.setDate(end.getDate() + 1);
    if (
      localDay(start) !== from ||
      localDay(new Date(end.getFullYear(), end.getMonth(), end.getDate() - 1)) !== to ||
      end <= start
    )
      throw new Error('日期区间无效。');
  }
  return {
    fromLocalDate: localDay(start),
    toLocalDateExclusive: localDay(end),
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    fromUtcMs: start.getTime(),
    toUtcMsExclusive: end.getTime(),
  };
}
/** datetime-local accepts nonexistent daylight-saving times; reject normalized values before import. */
export function localTimestamp(value: string): number {
  const date = new Date(value);
  const stamp = date.getTime();
  const roundTrip = `${localDay(date)}T${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
  if (
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value) ||
    !Number.isSafeInteger(stamp) ||
    stamp < 0 ||
    roundTrip !== value
  )
    throw new Error('开始时间无效，或该本地时间因夏令时切换而不存在。请选择真实的会议开始时间。');
  return stamp;
}
export const clock = (ms: number): string => {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 3600) ? `${Math.floor(s / 3600)}:` : ''}${String(Math.floor(s / 60) % 60).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
};
export const dateTime = (ms: number | null): string =>
  ms === null
    ? '时间未知'
    : new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium', timeStyle: 'short' }).format(ms);
