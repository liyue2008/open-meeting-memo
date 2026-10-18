(async () => {
  const sleep = (ms = 90) => new Promise((resolve) => setTimeout(resolve, ms));
  const send = (value) =>
    new Promise((resolve) => {
      window.__ack = resolve;
      window.webkit.messageHandlers.probe.postMessage(value);
    });
  const visible = (e) => !!e.getClientRects().length && !e.closest('[hidden]');
  const buttons = () => {
    const root = [...document.querySelectorAll('dialog[open]')].at(-1) || document;
    return [...root.querySelectorAll('button,a,[role=tab],summary')].filter(visible);
  };
  const find = (text) =>
    buttons().find((e) => e.textContent.trim() === text || e.getAttribute('aria-label') === text);
  const click = async (text) => {
    const element = find(text);
    if (!element || element.disabled) throw new Error('Missing enabled control: ' + text);
    element.focus();
    element.click();
    await sleep();
    return element;
  };
  const set = async (label, value) => {
    const element = [...document.querySelectorAll('input,select,textarea')].find(
      (e) =>
        visible(e) &&
        (e.getAttribute('aria-label') === label || e.closest('label')?.textContent.includes(label)),
    );
    if (!element) throw new Error('Missing field: ' + label);
    const setter = Object.getOwnPropertyDescriptor(
      element instanceof HTMLSelectElement
        ? HTMLSelectElement.prototype
        : HTMLInputElement.prototype,
      'value',
    ).set;
    setter.call(element, value);
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
    await sleep();
  };
  const measure = async (name, capture = false) => {
    await sleep();
    const initialScrollY = window.scrollY;
    const dialog = [...document.querySelectorAll('dialog[open]')].at(-1);
    const initialDialogScroll = dialog?.scrollTop || 0;
    const root = dialog || document.querySelector('main');
    const ignored = (e) => !!e.closest('.table-scroll,.markdown table');
    const outside = [...root.querySelectorAll('*')]
      .filter((e) => visible(e) && !ignored(e))
      .map((e) => ({ e, r: e.getBoundingClientRect() }))
      .filter(({ r }) => r.right > innerWidth + 1 || r.left < -1)
      .map(({ e, r }) => ({
        tag: e.tagName,
        className: String(e.className),
        right: Math.round(r.right),
        left: Math.round(r.left),
      }))
      .slice(0, 15);
    const controls = [...root.querySelectorAll('button,a,input,select,textarea,summary')].filter(
      (e) => visible(e) && !e.disabled,
    );
    const obscuredControls = [];
    if (window.__validationOnly || window.__recoveryOnly)
      for (const control of controls.filter((e) => !ignored(e))) {
        control.scrollIntoView({ block: 'center' });
        control.focus();
        const bounds = control.getBoundingClientRect();
        const x = (bounds.left + bounds.right) / 2,
          y = (bounds.top + bounds.bottom) / 2;
        const hit = document.elementFromPoint(x, y);
        if (!hit || (hit !== control && !control.contains(hit)))
          obscuredControls.push({ tag: control.tagName, type: control.type || '', x, y });
      }
    const last = controls.at(-1);
    if (dialog) dialog.scrollTop = dialog.scrollHeight;
    else window.scrollTo(0, document.documentElement.scrollHeight);
    const scrolledToBottom = dialog
      ? dialog.scrollHeight - dialog.clientHeight - dialog.scrollTop <= 1
      : document.documentElement.scrollHeight - innerHeight - window.scrollY <= 1;
    last?.scrollIntoView({ block: 'center' });
    last?.focus();
    await sleep(30);
    const r = last?.getBoundingClientRect();
    const lastControlInView =
      !r ||
      (r.top >= -1 && r.bottom <= innerHeight + 1 && r.left >= -1 && r.right <= innerWidth + 1);
    const toast = document.getElementById('toast');
    const save = document.querySelector('.settings-save');
    const overlap =
      toast && save
        ? Math.max(
            0,
            Math.min(toast.getBoundingClientRect().bottom, save.getBoundingClientRect().bottom) -
              Math.max(toast.getBoundingClientRect().top, save.getBoundingClientRect().top),
          )
        : 0;
    window.scrollTo(0, initialScrollY);
    if (dialog && !(window.__recoveryOnly && name.endsWith('external_unload_required')))
      dialog.scrollTop = initialDialogScroll;
    await send({
      name,
      capture,
      width: innerWidth,
      height: innerHeight,
      mode: window.__mode,
      scrollWidth: document.documentElement.scrollWidth,
      outside,
      controlCount: controls.length,
      lastControlInView,
      scrolledToBottom,
      guidanceLength:
        root.querySelector('textarea[aria-label="原生端人工处理指导"]')?.value.length || 0,
      guidanceScrolledToBottom: (() => {
        const field = root.querySelector('textarea[aria-label="原生端人工处理指导"]');
        return !field || field.scrollHeight - field.clientHeight - field.scrollTop <= 1;
      })(),
      focusInsideDialog: !dialog || dialog.contains(document.activeElement),
      dialogCount: document.querySelectorAll('dialog[open]').length,
      toastOpacity: toast ? getComputedStyle(toast).opacity : null,
      horizontalScrollX: window.scrollX,
      lastControlRect: r ? { top: r.top, bottom: r.bottom, left: r.left, right: r.right } : null,
      toastCloseEnabled: toast
        ? !!toast.querySelector('button') &&
          getComputedStyle(toast.querySelector('button')).pointerEvents !== 'none'
        : null,
      toastSaveOverlapPx: overlap,
      playCalls: window.__playCalls,
      tableScrolls: [...root.querySelectorAll('.table-scroll,.markdown table')].map((e) => ({
        clientWidth: e.clientWidth,
        scrollWidth: e.scrollWidth,
      })),
      alertCount: root.querySelectorAll('[role=alert]').length,
      obscuredControls,
      invalidResponses: [...window.__invalidResponses],
      bridgeFailures: [...window.__bridgeFailures],
      invalidBridgeData: document
        .querySelector('#root')
        .textContent.includes('桌面服务返回了无效或过大的数据'),
      missingDialogError: Boolean(
        window.__dialogsOnly &&
        name.endsWith('-error') &&
        (!dialog || !dialog.querySelector('[role=alert]')),
      ),
    });
    window.scrollTo(0, 0);
  };
  const remount = async (mode) => {
    window.__mode = mode;
    window.__remount();
    await sleep(150);
    window.scrollTo(0, 0);
  };
  const openDetail = async (mode) => {
    await remount(mode === 'empty' || mode === 'error' ? 'normal' : mode);
    const title = document.querySelector('.meeting-title');
    if (!title) throw new Error('Missing fixture meeting');
    window.__mode = mode;
    title.focus();
    title.click();
    await sleep(160);
  };
  let serial = 0;
  try {
    if (window.__stateSweep) {
      for (const [width, height, theme, sidebar] of [
        [800, 600, 'light', 'expanded'],
        [1280, 820, 'dark', 'collapsed'],
      ]) {
        await send({ resize: true, width, height });
        localStorage.setItem('omm.theme', theme);
        localStorage.setItem('omm.sidebar', sidebar);
        const prefix = `${width}-${height}-${theme}-${sidebar}-states`;
        if (window.__recoveryOnly) {
          const guidance =
            'Verify the original OMM tool and current-user job; complete work, use the verified original CLI service stop and wait; manually verify identity then /bin/launchctl bootout gui/501/org.openmeetingmemo.service. Keep the disk plist, recordings and configuration; do not use service uninstall. Return to inspect Switch, adopt stopped, then separately Start. ' +
            'SYNTHETIC-LONG-GUIDANCE '.repeat(24);
          const recovery = {
            state: 'unknown',
            canApply: false,
            planToken: null,
            expiresAtMs: null,
            effects: ['No control of unverified processes; no files changed'],
            guidance,
          };
          window.__scenario = {
            recordPrefs: { state: 'unknown', autoRecord: null, recordingConsent: null },
            configPrefs: { initialized: true, autoRecord: null, recordingConsent: null },
          };
          await remount('normal');
          await click('设置');
          await measure(`${prefix}-recovery-unknown-preferences`, true);
          if (
            document.querySelector('.recording-bar').textContent.includes('当前未录音') ||
            document.querySelector('.settings-summary').textContent.includes('自动录音：关')
          )
            throw new Error('Unknown preference presented as false');
          for (const reason of [
            'profile_requires_takeover',
            'external_stop_required',
            'external_unload_required',
            'registration_review_required',
            'incompatible_schema',
          ]) {
            window.__scenario = { recoveryPlan: { ...recovery, reasons: [reason] } };
            await remount('normal');
            await click('状态与诊断');
            await click('安全切换服务版本');
            const field = document.querySelector('dialog[open] textarea');
            if (!field || field.value !== guidance)
              throw new Error('Missing exact native guidance');
            field.scrollTop = field.scrollHeight;
            field.focus();
            field.select();
            await click('复制人工处理指导');
            if (window.__copiedGuidance !== guidance) throw new Error('Guidance copy differs');
            if (!find('确认安全切换').disabled) throw new Error('Unsafe confirmation enabled');
            await measure(`${prefix}-recovery-${reason}`, true);
          }
          window.__adopted = false;
          window.__scenario = {
            adoption: true,
            recoveryPlan: {
              state: 'stopped',
              reasons: ['profile_requires_takeover'],
              effects: ['Keep configuration, consent and database bytes unchanged; do not start'],
              guidance: 'Adopt only. Explicit Start is a separate confirmation.',
            },
          };
          await remount('normal');
          await click('状态与诊断');
          await click('安全切换服务版本');
          await measure(`${prefix}-recovery-adopt-confirm`, true);
          await click('确认采用（不启动）');
          await sleep(100);
          await measure(`${prefix}-recovery-adopted-stopped`, true);
          if (!document.getElementById('toast')?.textContent.includes('服务仍已停止'))
            throw new Error('Adoption claimed running');
          window.__scenario.failedCommands = ['service_apply'];
          await click('启动后台服务');
          await click('确认启动');
          await sleep(100);
          await measure(`${prefix}-recovery-start-failure`, true);
          if (!document.querySelector('dialog[open] [role=alert]') || !find('确认启动').disabled)
            throw new Error('Start failure not visible or plan reused');
          window.__scenario.failedCommands = ['service_plan'];
          await click('重新检查计划');
          await measure(`${prefix}-recovery-recheck-failure`, true);
          window.__scenario.failedCommands = [];
          await click('重新检查计划');
          await measure(`${prefix}-recovery-start-retry`, true);
          await click('确认启动');
          await measure(`${prefix}-recovery-started`, true);
          window.__scenario = {
            expired: true,
            recoveryPlan: {
              state: 'stopped',
              reasons: ['ready'],
              canApply: true,
              effects: [],
              guidance: null,
            },
          };
          await remount('normal');
          await click('状态与诊断');
          await click('启动后台服务');
          await measure(`${prefix}-recovery-expired`, true);
          if (
            !find('确认启动').disabled ||
            !document.querySelector('dialog[open]').textContent.includes('计划已过期')
          )
            throw new Error('Expired plan lacks feedback');
          window.__scenario = { failedCommands: ['record_status', 'config_get'] };
          await remount('normal');
          await click('设置');
          await measure(`${prefix}-recovery-read-failure`, true);
          serial++;
          continue;
        }
        if (window.__dialogsOnly) {
          window.__scenario = { readOnly: true, recordState: 'unknown' };
          await remount('normal');
          await measure(`${prefix}-readonly-unknown`, true);
          window.__scenario = { failedCommands: ['profile_initialize'] };
          await remount('empty');
          await click('状态与诊断');
          await click('初始化本机资料库');
          await click('确认初始化');
          await sleep(100);
          await measure(`${prefix}-initialize-error`, true);
          for (const label of ['启动后台服务', '停止后台服务', '安全切换服务版本']) {
            window.__scenario = { applyAllowed: true, failedCommands: ['service_apply'] };
            await remount('normal');
            await click('状态与诊断');
            await click(label);
            await click(
              label === '启动后台服务'
                ? '确认启动'
                : label === '停止后台服务'
                  ? '确认停止'
                  : '确认安全切换',
            );
            await sleep(100);
            await measure(`${prefix}-${label}-apply-error`, true);
          }
          window.__scenario = { failedCommands: ['record_auto_set'] };
          await remount('normal');
          await click('启用自动录音');
          await click('确认同意并启用');
          await sleep(100);
          await measure(`${prefix}-auto-error`, true);
          window.__scenario = { failedCommands: ['meeting_trash'] };
          await openDetail('normal');
          await click('移入回收站');
          await click('移入回收站', document.querySelector('[role=dialog]'));
          await sleep(100);
          await measure(`${prefix}-trash-error`, true);
          window.__scenario = { failedCommands: ['meeting_restore'] };
          await remount('normal');
          await set('位置', 'trash');
          document.querySelector('tbody input[type=checkbox]').click();
          await sleep();
          await click('恢复');
          await click('确认恢复');
          await sleep(100);
          await measure(`${prefix}-restore-error`, true);
          window.__scenario = { failedCommands: ['processing_cancel'] };
          await remount('normal');
          await click('处理队列');
          await click('第 1 次 · 转写');
          await click('取消任务');
          await click('确认取消');
          await sleep(100);
          await measure(`${prefix}-cancel-error`, true);
          window.__scenario = { pendingCommands: ['import_select'] };
          await remount('normal');
          await click('＋ 新建会议');
          await click('选择音频文件');
          await sleep(100);
          await measure(`${prefix}-import-busy`, true);
          window.__pending.shift()();
          await sleep(100);
          await measure(`${prefix}-import-error`, true);
          serial++;
          continue;
        }
        if (window.__validationOnly) {
          window.__scenario = { invalidConfig: true, invalidConfigCount: 10 };
          await remount('normal');
          await click('设置');
          await set('Pi 提供商', 'synthetic-invalid');
          await sleep(450);
          await measure(`${prefix}-config-invalid`, true);
          serial++;
          continue;
        }
        for (const state of ['recording', 'sealing', 'unknown']) {
          window.__scenario = { recordState: state, readOnly: state === 'unknown' };
          await remount('normal');
          await measure(`${prefix}-record-${state}`, true);
        }
        for (const state of [
          'recording',
          'unsealed',
          'queued',
          'transcribing',
          'transcript_committed',
          'summarizing',
          'blocked',
          'interrupted',
          'cancelled',
          'empty',
          'unknown',
        ]) {
          window.__scenario = {
            meetingStatus: state,
            taskStatus: [
              'queued',
              'transcribing',
              'summarizing',
              'blocked',
              'interrupted',
              'cancelled',
            ].includes(state)
              ? state
              : 'failed',
          };
          await remount('normal');
          await measure(`${prefix}-meeting-${state}`);
          await click('处理队列');
          await measure(`${prefix}-queue-${state}`);
        }
        for (const status of ['partial', 'invalid', 'historical']) {
          window.__scenario = { artifactStatus: status };
          await openDetail('long');
          for (const tab of ['会议纪要', '逐字稿']) {
            await click(tab);
            await measure(`${prefix}-artifact-${status}-${tab}`, true);
          }
        }
        window.__scenario = {
          pendingCommands: [
            'meeting_list',
            'task_list',
            'config_get',
            'diagnostics_get',
            'notification_history',
            'service_plan',
          ],
        };
        for (const page of ['会议库', '处理队列', '设置', '状态与诊断']) {
          await remount('normal');
          if (page !== '会议库') await click(page);
          await measure(`${prefix}-loading-${page}`, true);
        }
        window.__scenario = {};
        await remount('normal');
        await click('处理队列');
        window.__scenario = { failedCommands: ['task_get'] };
        await click('第 1 次 · 转写');
        await measure(`${prefix}-task-detail-error`, true);
        await click('关闭对话框');
        window.__scenario = { invalidConfig: true };
        await remount('normal');
        await click('设置');
        await set('Pi 提供商', 'synthetic-invalid');
        await sleep(450);
        await measure(`${prefix}-config-invalid`, true);
        window.__scenario = {};
        await remount('normal');
        await click('＋ 新建会议');
        window.__failAction = true;
        await click('选择音频文件');
        await measure(`${prefix}-import-select-error`, true);
        await click('关闭对话框');
        await remount('normal');
        await click('＋ 新建会议');
        await set('会议标题', '合成标题');
        const consent = document.querySelector('dialog input[type=checkbox]');
        consent.click();
        await sleep();
        window.__failAction = true;
        await click('开始录音');
        await measure(`${prefix}-manual-record-error`, true);
        await click('关闭对话框');
        await remount('normal');
        await click('＋ 新建会议');
        await set('会议标题', '合成导入标题');
        await click('选择音频文件');
        await set('原会议开始时间（本地）', '2026-01-01T12:00');
        window.__failAction = true;
        await click('导入');
        await measure(`${prefix}-import-commit-error`, true);
        await click('关闭对话框');
        serial++;
      }
      await send({
        name: 'summary',
        combinationsCompleted: serial,
        playCalls: window.__playCalls,
        capture: false,
      });
      await send({ done: true });
      return;
    }
    for (const [width, height] of [
      [800, 600],
      [1280, 820],
    ])
      for (const theme of ['light', 'dark'])
        for (const collapsed of [false, true]) {
          await send({ resize: true, width, height });
          localStorage.setItem('omm.theme', theme); // App's actual preference spelling is selected below too.
          await remount('normal');
          await set('外观主题', theme);
          const toggle = find(collapsed ? '收起侧栏' : '展开侧栏');
          if (toggle) {
            toggle.click();
            await sleep();
          }
          const prefix = `${width}-${height}-${theme}-${collapsed ? 'collapsed' : 'expanded'}`;
          const capture =
            (width === 800 && theme === 'light' && !collapsed) ||
            (width === 1280 && theme === 'dark' && collapsed);
          if (window.__diagnosticsOnly) {
            for (const mode of ['normal', 'empty', 'error', 'long']) {
              await remount(mode);
              await click('状态与诊断');
              await measure(`${prefix}-${mode}-diagnostics-validated`, capture);
            }
            serial++;
            continue;
          }
          for (const mode of ['normal', 'empty', 'error', 'long']) {
            for (const page of ['会议库', '处理队列', '设置', '状态与诊断', '使用帮助']) {
              await remount(mode);
              if (page !== '会议库') await click(page);
              await measure(
                `${prefix}-${mode}-${page}`,
                capture && (mode === 'normal' || mode === 'long'),
              );
              if (page === '设置' && mode !== 'error') {
                for (const legend of document.querySelectorAll('legend')) {
                  legend.scrollIntoView();
                  await measure(`${prefix}-${mode}-group-${legend.textContent}`, false);
                }
                await click('本机数据位置');
                await measure(`${prefix}-${mode}-group-paths`, capture && mode === 'long');
              }
            }
            await openDetail(mode);
            if (mode === 'error') {
              await measure(`${prefix}-${mode}-detail`, capture);
              continue;
            }
            for (const tab of ['会议纪要', '逐字稿', '处理版本']) {
              await click(tab);
              await measure(
                `${prefix}-${mode}-detail-${tab}`,
                capture && (mode === 'normal' || mode === 'long'),
              );
            }
          }
          // Every real modal is entered through the actual App controls, never a replacement component.
          for (const long of [false, true]) {
            const mode = long ? 'long' : 'normal';
            await openDetail(mode);
            for (const label of ['编辑', '重新处理', '移入回收站']) {
              await click(label);
              await measure(`${prefix}-${mode}-dialog-${label}`, capture);
              if (label === '编辑') {
                window.__failAction = true;
                await click('保存');
                await measure(`${prefix}-${mode}-dialog-edit-error`, capture);
              }
              if (label === '重新处理') {
                window.__failAction = true;
                await click('开始处理 1 场');
                await measure(`${prefix}-${mode}-dialog-reprocess-error`, capture);
              }
              await click('关闭对话框');
              window.__failAction = false;
            }
            await remount(mode);
            await set('位置', 'trash');
            const checkbox = document.querySelector('tbody input[type=checkbox]');
            checkbox.click();
            await sleep();
            await click('恢复');
            await measure(`${prefix}-${mode}-dialog-restore`, capture);
            window.__failAction = true;
            await click('确认恢复');
            await measure(`${prefix}-${mode}-dialog-restore-error`, false);
            await click('关闭对话框');
            await remount(mode);
            await click('＋ 新建会议');
            await measure(`${prefix}-${mode}-dialog-create`, capture);
            await click('选择音频文件');
            await measure(`${prefix}-${mode}-dialog-import-selected`, capture);
            await click('关闭对话框');
            await remount(mode);
            await click('启用自动录音');
            await measure(`${prefix}-${mode}-dialog-auto-consent`, capture);
            window.__failAction = true;
            await click('确认同意并启用');
            await measure(`${prefix}-${mode}-dialog-auto-error`, false);
            await click('关闭对话框');
            await remount(mode);
            await click('处理队列');
            await click('第 1 次 · 转写');
            await measure(`${prefix}-${mode}-dialog-task`, capture);
            await click('取消任务');
            await measure(`${prefix}-${mode}-dialog-task-cancel`, capture);
            await click('关闭对话框');
            await click('关闭对话框');
            await remount(mode);
            await click('设置');
            await set('Pi 提供商', 'synthetic-draft');
            await sleep(420);
            await click('重新读取配置');
            await measure(`${prefix}-${mode}-dialog-discard`, capture);
            await click('关闭对话框');
            await click('使用帮助');
            await measure(`${prefix}-${mode}-dialog-unsaved`, capture);
            await click('留在设置');
            window.__failAction = true;
            await click('保存配置');
            await measure(`${prefix}-${mode}-settings-save-conflict`, capture);
            await click('关闭提示');
          }
          for (const mode of ['normal', 'empty', 'unknown', 'busy']) {
            await remount(mode);
            await click('状态与诊断');
            if (mode === 'empty') {
              await click('初始化本机资料库');
              await measure(`${prefix}-dialog-initialize`, capture);
              await click('关闭对话框');
            }
            for (const label of ['启动后台服务', '停止后台服务', '安全切换服务版本']) {
              await click(label);
              await measure(
                `${prefix}-${mode}-dialog-service-${label}`,
                capture && mode !== 'normal',
              );
              await click('关闭对话框');
            }
          }
          serial++;
        }
    await send({
      name: 'summary',
      combinationsCompleted: serial,
      playCalls: window.__playCalls,
      capture: false,
    });
    await send({ done: true });
  } catch (error) {
    await send({
      name: 'driver-failure',
      error: String(error),
      bridgeFailures: window.__bridgeFailures,
      invalidResponses: window.__invalidResponses,
      calls: window.__calls,
      capture: true,
    });
    await send({ done: true });
  }
})();
