import { Alert, Button, Paper, Text, Textarea, Title } from '@mantine/core';
import { useEffect, useRef, useState } from 'react';
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
  busy: '停止或切换暂受保护',
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
  service_busy:
    '停止或切换暂受保护：有活动录音、未封存工作、处理任务或文件读取占用。请等待工作结束，或关闭会议详情释放读取后重新检查。',
  unowned_service: '当前服务归属不明。为保护已有工作，不能启动、停止或接管它。',
  bundle_unavailable: '当前 App 缺少经过验证的内置程序，不能执行服务操作。',
  service_unreachable: '无法核实当前服务，暂时不能改变其运行状态。',
  already_running: '后台服务已经在运行，无需再次启动。',
  already_stopped: '后台服务已经停止，无需再次停止。',
  incompatible_service: '当前服务版本不兼容。只能在归属可核实且没有活动工作时显式安全切换。',
  profile_requires_takeover:
    '资料库尚待显式采用，不是已证实的数据库版本不兼容。请检查安全切换计划；采用与启动是两次操作。',
  external_stop_required:
    '发现无法验证的旧服务。App不会向它查询状态或停服；先核实原工具确属本软件、工作已完成，再用确认过的原命令行程序正常停止并等待成功，按原生指导人工核查登记。',
  external_unload_required:
    '旧自启服务仍被系统加载，即使没有运行进程也不能采用。App不会卸载它；请先按原生指导核查归属，再仅从当前用户域卸载该服务任务，保留磁盘服务登记文件，回到App重新检查。',
  registration_review_required:
    '无法完整验证登记、归属或残留工作。保留录音、配置与服务登记文件，先人工核查；不要尝试一键接管。',
  incompatible_schema: '实际资料库或所有者版本不受当前版本支持；保留文件，不初始化覆盖或自动迁移。',
};
const adopting = (plan: ServicePlan) =>
  plan.action === 'switch' &&
  plan.state === 'stopped' &&
  plan.reasons.includes('profile_requires_takeover');
function PlanDetails({ plan }: { plan: ServicePlan }) {
  const action = useAction();
  return (
    <>
      {plan.effects.length > 0 && (
        <>
          <Title order={3}>本次操作的实际影响</Title>
          <ul>
            {plan.effects.map((effect, i) => (
              <li key={i}>{effect}</li>
            ))}
          </ul>
        </>
      )}
      {plan.guidance && (
        <>
          <Text component="p">
            人工步骤仅供查看或复制，不会自动运行命令。先核实原工具和当前用户域已加载的服务任务（job）确属本软件，且工作已安全完成；保留磁盘服务登记文件（plist）、录音和配置，不使用会删除恢复所需登记文件的
            service uninstall。
          </Text>
          <label>
            原生端人工处理指导
            <Textarea aria-label="原生端人工处理指导" readOnly rows={6} value={plan.guidance} />
          </label>
          <Button
            disabled={action.busy}
            onClick={() =>
              void action.run(async () => {
                if (!navigator.clipboard)
                  throw new Error('无法使用剪贴板；请在指导框内全选并复制。');
                await navigator.clipboard.writeText(plan.guidance!);
                return '人工处理指导已复制；请核查后自行操作。';
              })
            }
            type="button"
            variant="default"
          >
            复制人工处理指导
          </Button>
          {!!action.error && <ErrorNotice error={action.error} />}
        </>
      )}
    </>
  );
}
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
  // A busy operation plan can describe a healthy service holding an audio reader.
  // Keep the protection in settings, rather than treating it as a global health warning.
  if (
    !startup.error &&
    plan?.compatibility.state === 'compatible' &&
    plan.compatibility.writesAllowed &&
    (plan.state === 'running' ||
      (plan.state === 'busy' &&
        plan.compatibility.admissionMode === 'service' &&
        plan.reasons.length > 0 &&
        plan.reasons.every((reason) => reason === 'service_busy')))
  )
    return null;
  return (
    <Alert
      component="section"
      className="compatibility-banner"
      aria-label="本机首次设置与服务状态"
      color="orange"
      role="status"
      p="sm"
    >
      <div className="button-row">
        <Text size="sm" component="span">
          {startup.error
            ? '无法核实本机资料库与服务状态。请打开设置查看真实诊断。'
            : !plan
              ? '正在读取本机资料库与服务状态…'
              : plan.state === 'uninitialized'
                ? '首次使用：尚未初始化本机资料库。打开设置不会自动创建资料或启动录音。'
                : `后台服务：${stateLabel[plan.state]}。`}
        </Text>
        <Button className="service-entry" onClick={open} type="button" variant="default">
          {plan?.state === 'uninitialized' ? '打开首次设置' : '打开服务设置'}
        </Button>
      </div>
    </Alert>
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
  const [checked, setChecked] = useState(false);
  const [refreshFeedback, setRefreshFeedback] = useState('');
  const [clock, setClock] = useState(Date.now());
  useEffect(() => {
    const timer = setInterval(() => setClock(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const [preparing, setPreparing] = useState(false);
  const [prepareError, setPrepareError] = useState<unknown>(null);
  const lock = useRef(false);
  const busy = action.busy || preparing;
  const now = Math.max(clock, Date.now());
  const prepare = async (requested: ServiceAction) => {
    if (lock.current || action.busy) return;
    lock.current = true;
    setPreparing(true);
    setConsumed(true);
    setPrepareError(null);
    setChecked(false);
    action.clearError();
    try {
      const fresh = await bridge.call('service_plan', { action: requested });
      if (fresh.action !== requested) throw new Error('服务返回的计划动作不匹配，未执行任何操作。');
      setPlan(fresh);
      setConsumed(false);
      setChecked(true);
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
      const adoption = adopting(selected);
      const expected = selected.action === 'stop' || adoption ? 'stopped' : 'running';
      if (result.action !== selected.action || result.state !== expected)
        throw new Error(
          `服务尚未达到已确认的目标，实际状态：${stateLabel[result.state]}。请重新检查。`,
        );
      const config = await bridge.call('config_get', {}).catch(() => null);
      if (config) client.setQueryData(['config_get'], config);
      else void client.invalidateQueries({ queryKey: ['config_get'] });
      if (adoption)
        return '采用完成，服务仍已停止；下一步请另点“启动后台服务”。既有偏好与同意保持不变。';
      return `后台服务当前状态：${stateLabel[result.state]}。请查看下方逐项就绪诊断。`;
    });
    if (!success) await client.invalidateQueries();
    if (success) setPlan(null);
  };
  return (
    <Paper
      className="panel section-spaced"
      aria-label="首次设置与后台服务"
      withBorder
      p="md"
      component="section"
    >
      <Title order={2}>首次设置与后台服务</Title>
      <Text component="p">
        打开 App 只读取状态。初始化、启动、停止和切换均需你明确确认；退出 App 后后台服务继续运行。
      </Text>
      {startup.isPending ? (
        <Loading />
      ) : startup.error ? (
        <ErrorNotice error={startup.error} retry={() => void startup.refetch()} />
      ) : (
        <>
          <Text component="p">
            当前状态：<strong>{stateLabel[startup.data.state]}</strong> · 目标 App 版本：
            {startup.data.targetVersion}
          </Text>
          <ul>
            {startup.data.reasons.map((reason) => (
              <li key={reason}>{reasonLabel[reason]}</li>
            ))}
          </ul>
          <PlanDetails plan={startup.data} />
          {startup.data.state === 'uninitialized' && startup.data.canInitialize && (
            <Button
              className="primary"
              disabled={busy}
              onClick={() => {
                action.clearError();
                setInitialize(true);
              }}
              type="button"
              variant="filled"
            >
              初始化本机资料库
            </Button>
          )}
          {startup.data.state === 'uninitialized' && !startup.data.canInitialize && (
            <Alert className="notice" variant="light" role="status">
              当前目录不能初始化。保留已有文件；先按拒绝原因处理，不会自动覆盖或迁移。
            </Alert>
          )}
        </>
      )}
      <div className="button-row">
        <Button
          disabled={busy}
          onClick={() => void prepare('start')}
          type="button"
          variant="default"
        >
          启动后台服务
        </Button>
        <Button
          disabled={busy}
          onClick={() => void prepare('stop')}
          type="button"
          variant="default"
        >
          停止后台服务
        </Button>
        <Button
          disabled={busy}
          onClick={() => void prepare('switch')}
          type="button"
          variant="default"
        >
          安全切换服务版本
        </Button>
        <Button
          disabled={busy || startup.isFetching}
          onClick={() =>
            void startup
              .refetch()
              .then((result) =>
                setRefreshFeedback(
                  result.error
                    ? '重新读取失败，保留资料；请按错误提示重试。'
                    : '已重新读取服务状态；尚未执行任何服务操作。',
                ),
              )
          }
          type="button"
          variant="default"
        >
          {startup.isFetching ? '正在检查服务状态…' : '重新检查服务状态'}
        </Button>
      </div>
      {refreshFeedback && (
        <Text role="status" component="p">
          {refreshFeedback}
        </Text>
      )}
      <Text className="muted" component="p" c="dimmed">
        录音同意单独确认。这里不下载模型、不请求权限、不更改自动录音或同意设置、不读取 Pi
        凭据；启动后按既有自动录音偏好与同意设置运行，可能自动录音；仍需分别核对依赖和权限。
      </Text>
      {!!prepareError && !plan && <ErrorNotice error={prepareError} />}{' '}
      {!!action.error && !initialize && !plan && <ErrorNotice error={action.error} />}
      {initialize && (
        <Modal title="确认初始化本机资料库" close={() => setInitialize(false)} busy={busy}>
          <Text component="p">
            仅在配置、数据和状态目录都确认为空时，创建本机默认配置与写入所有者记录。已有资料或未知文件会拒绝初始化，保留原件。
          </Text>
          <Text component="p">
            初始化后自动录音与录音同意均为关闭；不会启动服务、录音、下载模型或请求权限。
          </Text>
          {!!action.error && <ErrorNotice error={action.error} />}
          <div className="modal-actions">
            <Button
              disabled={busy}
              onClick={() => setInitialize(false)}
              type="button"
              variant="default"
            >
              暂不初始化
            </Button>
            <Button
              className="primary"
              disabled={busy}
              onClick={() => void initializeProfile()}
              type="button"
              variant="filled"
            >
              确认初始化
            </Button>
          </div>
        </Modal>
      )}
      {plan && (
        <Modal
          title={adopting(plan) ? '采用资料库并更新停机登记（不启动）' : actionLabel[plan.action]}
          close={() => setPlan(null)}
          busy={busy}
        >
          <Text component="p">
            实际状态：{stateLabel[plan.state]} · 目标版本：{plan.targetVersion}
          </Text>
          {plan.canApply && (
            <ul>
              {plan.reasons.map((reason) => (
                <li key={reason}>{reasonLabel[reason]}</li>
              ))}
            </ul>
          )}
          <PlanDetails plan={plan} />
          {checked && (
            <Text role="status" component="p">
              已重新检查计划；请核对本次影响后再确认。
            </Text>
          )}
          {plan.expiresAtMs !== null && plan.expiresAtMs <= now && (
            <Text role="status" component="p">
              计划已过期；请重新检查，不会自动重放。
            </Text>
          )}
          {plan.expiresAtMs !== null && (
            <Text component="p">计划有效期至：{dateTime(plan.expiresAtMs)}</Text>
          )}
          <Text component="p">
            执行时原生端会再次核对资料库、配置、服务身份、包内程序和活动工作。状态变化会拒绝执行，需要重新检查与确认。
          </Text>
          {plan.action === 'stop' && (
            <Text component="p">
              只停止归属明确、没有活动录音或处理任务的兼容服务。等待已拥有的工作退出和封存完成，保留配置、音频与产物。
            </Text>
          )}
          {adopting(plan) && (
            <Text component="p">
              仅采用资料库并更新停机登记，不启动服务；保留资料、配置和同意，完成后另点启动后台服务。
            </Text>
          )}
          {plan.action === 'switch' && plan.canApply && !adopting(plan) && (
            <Text component="p">
              只有归属可验证且没有活动工作时，才安全停止旧服务并启动当前 App
              内置版本。失败会如实报告；不改既有自动录音偏好或录音同意，启动后按原设置运行，可能自动录音。
            </Text>
          )}
          {plan.action === 'start' && (
            <Text component="p">
              只启动当前 App
              内经过验证的后台程序。启动后按既有自动录音偏好与同意设置运行，可能自动录音；转写模型与
              Pi 的就绪情况分别检查。
            </Text>
          )}
          {!!prepareError && <ErrorNotice error={prepareError} />}{' '}
          {!!action.error && <ErrorNotice error={action.error} />}
          <div className="modal-actions">
            <Button disabled={busy} onClick={() => setPlan(null)} type="button" variant="default">
              暂不操作
            </Button>
            <Button
              disabled={busy}
              onClick={() => void prepare(plan.action)}
              type="button"
              variant="default"
            >
              重新检查计划
            </Button>
            <Button
              className="primary"
              disabled={
                busy ||
                consumed ||
                !plan.canApply ||
                !plan.planToken ||
                plan.expiresAtMs === null ||
                plan.expiresAtMs <= now
              }
              onClick={() => void apply()}
              type="button"
              variant="filled"
            >
              {adopting(plan) ? '确认采用（不启动）' : confirmation[plan.action]}
            </Button>
          </div>
          {!plan.canApply && (
            <Text role="status" component="p">
              不能执行：{plan.reasons.map((reason) => reasonLabel[reason]).join(' ')}{' '}
              不会自动改用其他启动、停止或接管方式。
            </Text>
          )}
          {consumed && (
            <Text role="status" component="p">
              本次计划已使用或失效，必须重新检查后才能再次确认。
            </Text>
          )}
        </Modal>
      )}
    </Paper>
  );
}
