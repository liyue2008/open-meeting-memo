import { describe, expect, it } from 'vitest';
import { createBridge, BridgeFailure, type Transport } from '../src/bridge/client';
import { DESKTOP_BUDGETS } from '../src/contracts.generated';
import { exchanges, sample } from './fixtures';
describe('bridge/client frozen real adapter', () => {
  for (const [i, exchange] of exchanges.entries())
    it(`adapts shared exchange ${i + 1}: ${exchange.command}`, async () => {
      const bridge = createBridge(async (command, { request }) => {
        expect(command).toBe(exchange.command);
        expect(request.payload).toEqual(exchange.request.payload);
        return { ...exchange.response, requestId: request.requestId };
      });
      if ('error' in exchange.response)
        await expect(
          bridge.call(exchange.command, exchange.request.payload as never),
        ).rejects.toBeInstanceOf(BridgeFailure);
      else
        await expect(
          bridge.call(exchange.command, exchange.request.payload as never),
        ).resolves.toEqual(exchange.response.data);
    });
  it('rejects invalid request before invoking native, including arbitrary paths and unsupported configuration', async () => {
    let invoked = false;
    const bridge = createBridge(async () => {
      invoked = true;
      return {};
    });
    await expect(bridge.call('meeting_get', { meetingId: '../secret' })).rejects.toThrow(
      '请求不符合',
    );
    await expect(
      bridge.call('config_validate', { patch: { autoRecord: true } } as never),
    ).rejects.toThrow('请求不符合');
    await expect(bridge.call('media_open', { path: '/tmp/secret' } as never)).rejects.toThrow(
      '请求不符合',
    );
    expect(invoked).toBe(false);
  });
  it('rejects wrong identity, versions, extra fields and excessive responses', async () => {
    for (const mutate of [
      (r: Record<string, unknown>) => {
        r.requestId = 'wrong-id';
      },
      (r: Record<string, unknown>) => {
        r.schemaVersion = 2;
      },
      (r: Record<string, unknown>) => {
        r.extra = true;
      },
      (r: Record<string, unknown>) => {
        r.data = 'x'.repeat(DESKTOP_BUDGETS.bridgeBodyBytes);
      },
    ]) {
      const bridge = createBridge(async (_, { request }) => {
        const r: Record<string, unknown> = {
          schemaVersion: 1,
          requestId: request.requestId,
          data: sample('desktop_info'),
        };
        mutate(r);
        return r;
      });
      await expect(bridge.call('desktop_info', {})).rejects.toBeInstanceOf(BridgeFailure);
    }
  });
  it('redacts raw transport exceptions and forbids non-local or path-bearing media grants', async () => {
    const bridge = createBridge(async () => {
      throw new Error('/private/personal credential data');
    });
    await expect(bridge.call('desktop_info', {})).rejects.toThrow('无法连接桌面服务');
    for (const url of [
      'file:///tmp/secret.wav',
      'https://example.test/audio',
      'omm-media://elsewhere/grant',
      'omm-media://localhost/a/../secret',
      'omm-media://localhost/grant?path=secret',
    ]) {
      const media = createBridge(async (_, { request }) => ({
        schemaVersion: 1,
        requestId: request.requestId,
        data: { ...sample('media_open'), url },
      }));
      await expect(
        media.call(
          'media_open',
          exchanges.find((e) => e.command === 'media_open')!.request.payload as never,
        ),
      ).rejects.toBeInstanceOf(BridgeFailure);
    }
  });
  it('enforces four in-flight requests without race when waiters and fresh calls compete', async () => {
    const release: Array<() => void> = [];
    let running = 0;
    let peak = 0;
    const transport: Transport = async (_, { request }) => {
      running++;
      peak = Math.max(peak, running);
      await new Promise<void>((resolve) => release.push(resolve));
      running--;
      return { schemaVersion: 1, requestId: request.requestId, data: sample('desktop_info') };
    };
    const bridge = createBridge(transport);
    const pending = Array.from({ length: 12 }, () => bridge.call('desktop_info', {}));
    expect(release.length).toBe(4);
    for (let i = 0; i < 12; i++) {
      release[i]!();
      await new Promise((r) => setTimeout(r, 0));
      if (i === 0) pending.push(bridge.call('desktop_info', {}));
    }
    release[12]!();
    await Promise.all(pending);
    expect(peak).toBe(4);
  });
  it('rejects UTF-8 note overflow before native and binds the media URL to its exact grant/revision', async () => {
    let calls = 0;
    const bridge = createBridge(async () => {
      calls++;
      return {};
    });
    const update = structuredClone(
      exchanges.find((e) => e.command === 'meeting_update')!.request.payload,
    ) as { metadata: { note: string } };
    update.metadata.note = '汉'.repeat(5500);
    await expect(bridge.call('meeting_update', update as never)).rejects.toThrow('UTF-8');
    expect(calls).toBe(0);
    for (const changed of [{ grantId: 'fixture_other_000001' }, { revision: 'c'.repeat(64) }]) {
      const invalid = createBridge(async (_, { request }) => ({
        schemaVersion: 1,
        requestId: request.requestId,
        data: { ...sample('media_open'), ...changed },
      }));
      await expect(
        invalid.call(
          'media_open',
          exchanges.find((e) => e.command === 'media_open')!.request.payload as never,
        ),
      ).rejects.toBeInstanceOf(BridgeFailure);
    }
  });
  it('removes aborted queue waiters without consuming or leaking a slot', async () => {
    const release: Array<() => void> = [];
    const commands: string[] = [];
    const bridge = createBridge(async (command, { request }) => {
      commands.push(command);
      await new Promise<void>((resolve) => release.push(resolve));
      return { schemaVersion: 1, requestId: request.requestId, data: sample('desktop_info') };
    });
    const first = Array.from({ length: 4 }, () => bridge.call('desktop_info', {}));
    const controller = new AbortController();
    const cancelled = bridge.call('desktop_info', {}, controller.signal);
    controller.abort();
    await expect(cancelled).rejects.toThrow('请求已取消');
    expect(commands).toHaveLength(4);
    for (const done of release) done();
    await Promise.all(first);
    const fresh = bridge.call('desktop_info', {});
    expect(commands).toHaveLength(5);
    release[4]!();
    await expect(fresh).resolves.toEqual(sample('desktop_info'));
  });
  it('drops cancelled reads but never sends a persisted task cancellation', async () => {
    const controller = new AbortController();
    controller.abort();
    let calls = 0;
    const bridge = createBridge(async () => {
      calls++;
      return {};
    });
    await expect(bridge.call('desktop_info', {}, controller.signal)).rejects.toThrow('请求已取消');
    expect(calls).toBe(0);
  });
});
