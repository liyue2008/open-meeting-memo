import { useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useAction, useBridge } from '../app/context';
import { dateTime } from '../app/dates';
import { ErrorNotice, Loading, Modal } from '../components/ui';
import type {
  ServiceAction,
  ServiceLifecycleState,
  ServicePlan,
  ServicePlanReason,
} from '../contracts.generated';

const stateLabel: Record<ServiceLifecycleState, string> = {
  uninitialized: '尚未初始化',
  stopped: '已停止',
  running: '正在运行',
  incompatible: '版本不兼容',
  busy: '正在录音或处理任务',
  unknown: '状态未知',
};
const actionLabel: Record<ServiceAction, string> = {
  start: '启动后台服务',
  stop: '停止后台服务',
  switch: '安全切换到当前 App 版本',
};
const confirmation: Record<ServiceAction, string> = {
  start: '确认启动',
  stop: '确认停止',
  switch: '确认安全切换',
};
const reasonLabel: Record<ServicePlanReason, string> = {
  ready: '本次服务操作检查通过；模型、权限与 Pi 就绪情况另见下方诊断。',
  profile_not_initialized: '尚未初始化本机资料库。请先明确确认首次设置。',
  profile_not_empty: '资料目录已有内容，不能作为空资料库初始化。保留原资料并检查归属。',
  service_busy: '有活动录音、未封存工作或正在处理的任务；等待工作安全结束后重新检查。',
  unowned_service: '当前服务归属不明。为保护已有工作，不能启动、停止或接管它。',
  bundle_unavailable: '当前 App 缺少经过验证的内置程序，不能执行服务操作。',
  service_unreachable: '无法核实当前服务，暂时不能改变其运行状态。',
  already_running: '后台服务已经在运行，无需再次启动。',
  already_stopped: '后台服务已经停止，无需再次停止。',
  incompatible_service: '当前服务版本不兼容。只能在归属可核实且没有活动工作时显式安全切换。',
};
function useStartupPlan() {
  const bridge = useBridge();
  return useQuery({
    queryKey: ['service_plan', 'start'],
    queryFn: ({ signal }) => bridge.call('service_plan', { action: 'start' }, signal),
    refetchInterval: 10000,
  });
}
export function ServiceBanner({ open }: { open: () => void }) {
  const startup = useStartupPlan();
  const plan = startup.data;
  if (plan?.state === 'running' && plan.compatibility.writesAllowed) return null;
  return (
    <section className="compatibility-banner" aria-label="本机首次设置与服务状态">
      <span>
        {startup.error
          ? '无法核实本机资料库与服务状态。请打开设置查看真实诊断。'
          : !plan
            ? '正在读取本机资料库与服务状态…'
            : plan.state === 'uninitialized'
              ? '首次使用：尚未初始化本机资料库。打开设置不会自动创建资料或启动录音。'
              : `后台服务：${stateLabel[plan.state]}。请查看可执行的服务操作和逐项就绪诊断。`}
      </span>
      <button className="service-entry" onClick={open}>
        {plan?.state === 'uninitialized' ? '打开首次设置' : '打开服务设置'}
      </button>
    </section>
  );
}

export function ServiceLifecycle() {
  const bridge = useBridge();
  const client = useQueryClient();
  const startup = useStartupPlan();
  const action = useAction();
  const [initialize, setInitialize] = useState(false);
  const [plan, setPlan] = useState<ServicePlan | null>(null);
  const [consumed, setConsumed] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [prepareError, setPrepareError] = useState<unknown>(null);
  const lock = useRef(false);
  const busy = action.busy || preparing;
  const prepare = async (requested: ServiceAction) => {
    if (lock.current || action.busy) return;
    lock.current = true;
    setPreparing(true);
    setConsumed(true);
    setPrepareError(null);
    action.clearError();
    try {
      const fresh = await bridge.call('service_plan', { action: requested });
      if (fresh.action !== requested) throw new Error('服务返回的计划动作不匹配，未执行任何操作。');
      setPlan(fresh);
      setConsumed(false);
    } catch (error) {
      setPrepareError(error);
    } finally {
      lock.current = false;
      setPreparing(false);
    }
  };
  const initializeProfile = async () => {
    const success = await action.run(async () => {
      const snapshot = await bridge.call('profile_initialize', {});
      if (!snapshot.initialized) throw new Error('资料库仍未初始化。请查看真实诊断后重新检查。');
      const current = await bridge.call('config_get', {});
      if (!current.initialized)
        throw new Error('首次设置完成后仍无法读取已初始化的配置，请重新检查真实状态。');
      client.setQueryData(['config_get'], current);
      setInitialize(false);
      return '本机资料库已初始化。请检查配置和依赖，再明确启动后台服务。';
    });
    if (!success) await client.invalidateQueries();
  };
  const apply = async () => {
    if (!plan || consumed) return;
    const selected = plan;
    if (
      !selected.canApply ||
      !selected.planToken ||
      selected.expiresAtMs === null ||
      selected.expiresAtMs <= Date.now()
    ) {
      setConsumed(true);
      setPrepareError(new Error('本次计划不可执行或已过期。请重新检查；不会自动重放操作。'));
      return;
    }
    setConsumed(true);
    const success = await action.run(async () => {
      const result = await bridge.call('service_apply', { planToken: selected.planToken! });
      const expected = selected.action === 'stop' ? 'stopped' : 'running';
      if (result.action !== selected.action || result.state !== expected)
        throw new Error(
          `服务尚未达到已确认的目标，实际状态：${stateLabel[result.state]}。请重新检查。`,
        );
      const config = await bridge.call('config_get', {});
      client.setQueryData(['config_get'], config);
      return `后台服务当前状态：${stateLabel[result.state]}。请查看下方逐项就绪诊断。`;
    });
    if (!success) await client.invalidateQueries();
    if (success) setPlan(null);
  };
  return (
    <section className="panel section-spaced" aria-label="首次设置与后台服务">
      <h2>首次设置与后台服务</h2>
      <p>
        打开 App 只读取状态。初始化、启动、停止和切换均需你明确确认；退出 App 后后台服务继续运行。
      </p>
      {startup.isPending ? (
        <Loading />
      ) : startup.error ? (
        <ErrorNotice error={startup.error} retry={() => void startup.refetch()} />
      ) : (
        <>
          <p>
            当前状态：<strong>{stateLabel[startup.data.state]}</strong> · 目标 App 版本：
            {startup.data.targetVersion}
          </p>
          <ul>
            {startup.data.reasons.map((reason) => (
              <li key={reason}>{reasonLabel[reason]}</li>
            ))}
          </ul>
          {startup.data.state === 'uninitialized' && startup.data.canInitialize && (
            <button
              className="primary"
              disabled={busy}
              onClick={() => {
                action.clearError();
                setInitialize(true);
              }}
            >
              初始化本机资料库
            </button>
          )}
          {startup.data.state === 'uninitialized' && !startup.data.canInitialize && (
            <p className="notice">
              当前目录不能初始化。保留已有文件；先按拒绝原因处理，不会自动覆盖或迁移。
            </p>
          )}
        </>
      )}
      <div className="button-row">
        <button disabled={busy} onClick={() => void prepare('start')}>
          启动后台服务
        </button>
        <button disabled={busy} onClick={() => void prepare('stop')}>
          停止后台服务
        </button>
        <button disabled={busy} onClick={() => void prepare('switch')}>
          安全切换服务版本
        </button>
        <button disabled={busy || startup.isFetching} onClick={() => void startup.refetch()}>
          重新检查服务状态
        </button>
      </div>
      <p className="muted">
        录音同意单独确认。这里不下载模型、不请求权限、不更改自动录音或同意设置、不读取 Pi
        凭据；服务启动后仍需分别核对依赖和权限。
      </p>
      {!!prepareError && !plan && <ErrorNotice error={prepareError} />}{' '}
      {!!action.error && !initialize && !plan && <ErrorNotice error={action.error} />}
      {initialize && (
        <Modal title="确认初始化本机资料库" close={() => setInitialize(false)} busy={busy}>
          <p>
            仅在配置、数据和状态目录都确认为空时，创建本机默认配置与写入所有者记录。已有资料或未知文件会拒绝初始化，保留原件。
          </p>
          <p>初始化后自动录音与录音同意均为关闭；不会启动服务、录音、下载模型或请求权限。</p>
          {!!action.error && <ErrorNotice error={action.error} />}
          <div className="modal-actions">
            <button disabled={busy} onClick={() => setInitialize(false)}>
              暂不初始化
            </button>
            <button className="primary" disabled={busy} onClick={() => void initializeProfile()}>
              确认初始化
            </button>
          </div>
        </Modal>
      )}
      {plan && (
        <Modal title={actionLabel[plan.action]} close={() => setPlan(null)} busy={busy}>
          <p>
            实际状态：{stateLabel[plan.state]} · 目标版本：{plan.targetVersion}
          </p>
          <ul>
            {plan.reasons.map((reason) => (
              <li key={reason}>{reasonLabel[reason]}</li>
            ))}
          </ul>
          {plan.expiresAtMs !== null && <p>计划有效期至：{dateTime(plan.expiresAtMs)}</p>}
          <p>
            执行时原生端会再次核对资料库、配置、服务身份、包内程序和活动工作。状态变化会拒绝执行，需要重新检查与确认。
          </p>
          {plan.action === 'stop' && (
            <p>
              只停止归属明确、没有活动录音或处理任务的兼容服务。等待已拥有的工作退出和封存完成，保留配置、音频与产物。
            </p>
          )}
          {plan.action === 'switch' && (
            <p>
              只有归属可验证且没有活动工作时，才安全停止旧服务并启动当前 App
              内置版本。失败会如实报告，不自动恢复录音，不改录音同意。
            </p>
          )}
          {plan.action === 'start' && (
            <p>只启动当前 App 内经过验证的后台程序。录音、转写模型与 Pi 的就绪情况分别检查。</p>
          )}
          {!!prepareError && <ErrorNotice error={prepareError} />}{' '}
          {!!action.error && <ErrorNotice error={action.error} />}
          <div className="modal-actions">
            <button disabled={busy} onClick={() => setPlan(null)}>
              暂不操作
            </button>
            <button disabled={busy} onClick={() => void prepare(plan.action)}>
              重新检查计划
            </button>
            <button
              className="primary"
              disabled={
                busy ||
                consumed ||
                !plan.canApply ||
                !plan.planToken ||
                plan.expiresAtMs === null ||
                plan.expiresAtMs <= Date.now()
              }
              onClick={() => void apply()}
            >
              {confirmation[plan.action]}
            </button>
          </div>
          {!plan.canApply && (
            <p role="status">当前不能执行此操作；不会自动改用其他启动、停止或接管方式。</p>
          )}
          {consumed && <p role="status">本次计划已使用或失效，必须重新检查后才能再次确认。</p>}
        </Modal>
      )}
    </section>
  );
}
