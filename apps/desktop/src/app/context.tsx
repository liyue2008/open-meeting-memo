import { createContext, useContext, useState, type ReactNode } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { BridgeFailure, errorText, type Bridge } from '../bridge/client';
import type { BridgeError, Capability, WriteAdmission } from '../contracts.generated';
export const BridgeContext = createContext<Bridge | null>(null);
export function useBridge() {
  const bridge = useContext(BridgeContext);
  if (!bridge) throw new Error('Missing bridge provider');
  return bridge;
}
export function useInfo() {
  const bridge = useBridge();
  return useQuery({
    queryKey: ['desktop_info'],
    queryFn: ({ signal }) => bridge.call('desktop_info', {}, signal),
    refetchInterval: 10000,
  });
}
export function compatibilityReasonText(reason: BridgeError['reason']): string {
  if (reason === 'profile_requires_takeover')
    return '资料库尚待显式采用；缺少桌面所有者记录不是已证实的数据库版本不兼容。';
  if (reason === 'incompatible_schema')
    return '实际资料库或所有者版本不受支持；保留现有文件，不初始化覆盖。';
  if (reason === 'incompatible_service') return '当前服务版本或归属不能验证；不能控制未知旧服务。';
  return '尚未获得写入授权；请核查服务状态与原生人工指导。';
}
export function useAdmission() {
  const info = useInfo();
  const bridge = useBridge();
  return {
    compatibility: info.data?.compatibility,
    async obtain(capability?: Capability): Promise<WriteAdmission> {
      const current = await bridge.call('desktop_info', {});
      const c = current.compatibility;
      if (
        !c.writesAllowed ||
        !c.writeAdmission ||
        (capability && !c.capabilities.includes(capability))
      )
        throw new BridgeFailure({
          code: 'unavailable',
          reason: c.reason,
          message: `当前服务或资料库不允许此操作。${compatibilityReasonText(c.reason)}`,
          retryable: false,
          fieldErrors: [],
        });
      return c.writeAdmission;
    },
  };
}
const NoticeContext = createContext<(text: string) => void>(() => undefined);
export const useNotice = () => useContext(NoticeContext);
export function Notices({ children }: { children: ReactNode }) {
  const [notice, setNotice] = useState('');
  return (
    <NoticeContext.Provider value={setNotice}>
      {children}
      {notice && (
        <div id="toast" className="visible" role="status" aria-live="polite" aria-atomic="true">
          <span className="notice-copy">{notice}</span>
          <button aria-label="关闭提示" onClick={() => setNotice('')}>
            ×
          </button>
        </div>
      )}
    </NoticeContext.Provider>
  );
}
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const client = useQueryClient();
  const notice = useNotice();
  // Ref-like lock avoids duplicate activation before React has rendered disabled controls.
  const [lock] = useState(() => ({ held: false }));
  return {
    busy,
    error,
    clearError: () => setError(null),
    async run(operation: () => Promise<string | void>): Promise<boolean> {
      if (lock.held) return false;
      lock.held = true;
      setBusy(true);
      setError(null);
      try {
        const message = await operation();
        if (message) notice(message);
        await client.invalidateQueries();
        return true;
      } catch (e) {
        setError(e);
        notice(errorText(e));
        return false;
      } finally {
        lock.held = false;
        setBusy(false);
      }
    },
  };
}
