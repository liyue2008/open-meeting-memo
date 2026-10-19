// Test-only transports. Production imports never reach this directory.
import fs from 'node:fs';
import path from 'node:path';
import { createBridge, type Transport } from '../src/bridge/client';
import type {
  BridgeError,
  DesktopCommand,
  DesktopCommandMap,
  MeetingRow,
  TaskRow,
  ImportSelection,
} from '../src/contracts.generated';
const folder = path.resolve(process.cwd(), '../../contracts/desktop/v1/examples');
export interface Exchange {
  command: DesktopCommand;
  request: { schemaVersion: 1; requestId: string; payload: unknown };
  response: { schemaVersion: 1; requestId: string; data?: unknown; error?: unknown };
}
export const exchanges: Exchange[] = fs
  .readdirSync(folder)
  .filter((n) => n.endsWith('.json'))
  .flatMap(
    (n) => JSON.parse(fs.readFileSync(path.join(folder, n), 'utf8')) as Exchange | Exchange[],
  );
export function sample<C extends DesktopCommand>(command: C): DesktopCommandMap[C]['response'] {
  return structuredClone(
    exchanges.find((e) => e.command === command && 'data' in e.response)!.response.data,
  ) as DesktopCommandMap[C]['response'];
}
export const meetingFixture = (): MeetingRow => ({
  ...sample('meeting_get').meeting,
  metadata: { title: '合成设计讨论', note: '仅供测试', tags: ['合成'] },
  durationMs: 60000,
  startedAtMs: new Date().getTime(),
  status: 'completed',
  source: 'imported',
  capabilities: { edit: true, trash: true, restore: false, reprocess: true, reason: null },
});
export const selectionFixture = (): ImportSelection => ({
  selectionHandle: 'fixture_selection_000001',
  displayName: 'synthetic.wav',
  byteLength: 32044,
  format: 'wav',
  durationMs: 1000,
  channels: 1,
  sampleRate: 16000,
  expiresAtMs: 1893456000000,
});
export const taskFixture = (): TaskRow => ({
  ...sample('task_get').task,
  status: 'failed',
  canRetry: true,
  canCancel: true,
  canRequeue: true,
});
export class NativeFailure extends Error {
  constructor(public readonly detail: BridgeError) {
    super(detail.message);
  }
}
export const conflict = () =>
  new NativeFailure({
    code: 'conflict',
    reason: 'stale_revision',
    message: '配置已被其他进程修改，草稿保留。',
    retryable: false,
    fieldErrors: [],
  });
type Overrides = {
  [C in DesktopCommand]?: (
    payload: DesktopCommandMap[C]['request'],
  ) => DesktopCommandMap[C]['response'] | Promise<DesktopCommandMap[C]['response']>;
};
export function fixtureBridge(overrides: Overrides = {}) {
  const calls: Array<{ command: DesktopCommand; payload: unknown }> = [];
  const transport: Transport = async (command, { request }) => {
    calls.push({ command, payload: structuredClone(request.payload) });
    const override = overrides[command] as ((payload: unknown) => unknown) | undefined;
    try {
      const data = override ? await override(request.payload) : sample(command);
      return { schemaVersion: 1, requestId: request.requestId, data };
    } catch (e) {
      if (e instanceof NativeFailure)
        return { schemaVersion: 1, requestId: request.requestId, error: e.detail };
      throw e;
    }
  };
  return { bridge: createBridge(transport), calls };
}
