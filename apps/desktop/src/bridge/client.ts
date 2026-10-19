import { invoke } from '@tauri-apps/api/core';
import * as validators from './validators.generated.js';
import {
  DESKTOP_BUDGETS,
  type DesktopCommand,
  type DesktopCommandMap,
  type Request,
  type Response,
  type BridgeError,
} from '../contracts.generated';

export class BridgeFailure extends Error {
  constructor(public readonly detail: BridgeError) {
    super(detail.message);
    this.name = 'BridgeFailure';
  }
}
export type Transport = (
  command: DesktopCommand,
  args: { request: Request<unknown> },
) => Promise<unknown>;
export interface Bridge {
  call<C extends DesktopCommand>(
    command: C,
    payload: DesktopCommandMap[C]['request'],
    signal?: AbortSignal,
  ): Promise<DesktopCommandMap[C]['response']>;
}
function valid(name: string, value: unknown): boolean {
  const validator = validators[name as keyof typeof validators];
  return typeof validator === 'function' && validator(value);
}
const failure = (message: string, code: BridgeError['code'] = 'corrupt') =>
  new BridgeFailure({ code, reason: null, retryable: false, message, fieldErrors: [] });
const byteLength = (value: unknown) => new TextEncoder().encode(JSON.stringify(value)).length;
const abort = () => new DOMException('请求已取消', 'AbortError');

/** The sole production invoke boundary. Cancellation drops obsolete reads; it never cancels a persisted task. */
export function createBridge(
  transport: Transport = (command, args) => invoke(command, args),
): Bridge {
  let active = 0;
  const waiters: Array<() => void> = [];
  return {
    async call(command, payload, signal) {
      const request = { schemaVersion: 1 as const, requestId: crypto.randomUUID(), payload };
      if (
        !valid(`${command}_request`, request) ||
        byteLength(request) > DESKTOP_BUDGETS.mutationJsonBytes
      )
        throw failure('请求不符合桌面接口或超出大小上限。', 'invalid_input');
      if (command === 'meeting_update' || command === 'import_commit') {
        const metadata = (payload as DesktopCommandMap['meeting_update']['request']).metadata;
        if (new TextEncoder().encode(metadata.note).length > DESKTOP_BUDGETS.noteUtf8Bytes)
          throw failure('备注超过 16 KiB 的 UTF-8 字节上限。', 'invalid_input');
      }
      if (signal?.aborted) throw abort();
      if (active >= DESKTOP_BUDGETS.blockingRequestsPerWindow) {
        await new Promise<void>((resolve, reject) => {
          const ready = () => {
            active++;
            clearTimeout(timer);
            signal?.removeEventListener('abort', cancelled);
            resolve();
          };
          const remove = () => {
            const index = waiters.indexOf(ready);
            if (index >= 0) waiters.splice(index, 1);
          };
          const cancelled = () => {
            remove();
            clearTimeout(timer);
            reject(abort());
          };
          const timer = setTimeout(() => {
            remove();
            signal?.removeEventListener('abort', cancelled);
            reject(failure('请求队列繁忙，请稍后重试。', 'timeout'));
          }, 15000);
          waiters.push(ready);
          signal?.addEventListener('abort', cancelled, { once: true });
        });
      } else active++;
      try {
        if (signal?.aborted) throw abort();
        let raw: unknown;
        try {
          raw = await transport(command, { request });
        } catch {
          throw failure('无法连接桌面服务，请查看状态与诊断。', 'unavailable');
        }
        if (signal?.aborted) throw abort();
        if (byteLength(raw) > DESKTOP_BUDGETS.bridgeBodyBytes || !valid(`${command}_response`, raw))
          throw failure('桌面服务返回了无效或过大的数据。');
        const response = raw as Response<DesktopCommandMap[typeof command]['response']>;
        if (response.requestId !== request.requestId) throw failure('桌面响应标识不匹配。');
        if ('error' in response) throw new BridgeFailure(response.error);
        // Native verifies grants again. Renderer never assigns arbitrary URLs to the media element.
        if (command === 'media_open') {
          const grant = response.data as DesktopCommandMap['media_open']['response'];
          const requested = payload as DesktopCommandMap['media_open']['request'];
          if (
            !/^omm-media:\/\/localhost\/[A-Za-z0-9_-]+$/.test(grant.url) ||
            grant.url !== `omm-media://localhost/${grant.grantId}` ||
            grant.revision !== requested.expectedRevision
          )
            throw failure('录音地址或版本不符合受限媒体协议。');
        }
        return response.data;
      } finally {
        active--;
        waiters.shift()?.();
      }
    },
  };
}
export function errorText(error: unknown): string {
  return error instanceof BridgeFailure
    ? `${error.message}${error.detail.reason ? `（${error.detail.reason}）` : ''}`
    : error instanceof Error
      ? error.message
      : '操作未完成，请重试或查看诊断。';
}
