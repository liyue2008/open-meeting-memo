import { Alert, Button, Group, Paper, Text, Title } from '@mantine/core';
import { useRef, useState } from 'react';
import { useBridge } from '../app/context';
import { ErrorNotice } from '../components/ui';
import type { PermissionResult } from '../contracts.generated';
const micLabel = { authorized: '已授权', denied: '已拒绝', restricted: '受限制', not_determined: '尚未决定', unknown: '未知' };
const outcomeLabel = { completed: '请求调用已结束', unconfirmed: '无法确认请求结果', timed_out: '等待已超时', cancelled: '等待已取消', cleanup_failed: '权限资源清理失败，录音被阻止；可正常退出服务后检查' };
export function CapturePermissions() {
  const bridge = useBridge();
  const operation = useRef<string | null>(null);
  const [pending, setPending] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [result, setResult] = useState<PermissionResult | null>(null);
  const [error, setError] = useState<unknown>(null);
  async function request() {
    if (operation.current) return;
    const id = crypto.randomUUID(); operation.current = id;
    setPending(true); setResult(null); setError(null);
    try {
      const value = await bridge.call('permissions_request', { action: 'request', operationId: id });
      if (operation.current === id) setResult(value);
    } catch (failure) { if (operation.current === id) setError(failure); }
    finally { if (operation.current === id) { operation.current = null; setPending(false); } }
  }
  async function cancel() {
    const id = operation.current; if (!id || cancelling) return;
    setCancelling(true);
    try {
      const value = await bridge.call('permissions_request', { action: 'cancel', operationId: id });
      setResult(value);
    } catch (failure) { setError(failure); }
    finally { setCancelling(false); }
  }
  return <Paper className="panel section-spaced" withBorder p="md" component="section" aria-label="录音权限">
    <Title order={2}>录音权限</Title>
    <Text component="p">先启动兼容后台服务。仅点击才正常申请系统权限；不录音、不写 WAV、不创建任务。</Text>
    <Group><Button type="button" disabled={pending || cancelling} onClick={() => void request()}>申请录音权限</Button>
      {pending && <Button type="button" variant="default" disabled={cancelling} onClick={() => void cancel()}>取消等待</Button>}</Group>
    {pending && <Text role="status">正在等待系统授权结果；活动录音或未完成的授权会拒绝新请求。</Text>}
    {error != null && <ErrorNotice error={error} />}
    {result && <Alert role="status"><Text>麦克风：{micLabel[result.microphone]}；系统音频：未知</Text><Text>{outcomeLabel[result.outcome]}</Text></Alert>}
    <Text component="p">取消或超时不能关闭系统同意框；OS 操作尚未结束时仍保持忙碌。授权或调用成功不代表已有声音，录音恢复尚未验证。无需手工添加内部 exe。</Text>
  </Paper>;
}
