// Current Mantine UI, copied React + strict synthetic bridge. Never invokes play.
(async () => {
  const sleep = (ms = 100) => new Promise((resolve) => setTimeout(resolve, ms));
  const send = (value) =>
    new Promise((resolve) => {
      window.__ack = resolve;
      window.webkit.messageHandlers.probe.postMessage(value);
    });
  const labels = ['会议库', '处理队列', '设置', '状态与诊断', '使用帮助'];
  const visible = (element) => {
    const rect = element.getBoundingClientRect(),
      style = getComputedStyle(element);
    return (
      rect.width > 1 &&
      rect.height > 1 &&
      style.display !== 'none' &&
      style.visibility !== 'hidden' &&
      style.opacity !== '0'
    );
  };
  const button = (text, root = dialog() || document) =>
    [...root.querySelectorAll('button,a')].find(
      (element) =>
        element.textContent.trim().replace(/^[＋+]\s*/, '') === text ||
        element.getAttribute('aria-label') === text,
    );
  const click = async (text, root = dialog() || document) => {
    const element = button(text, root);
    if (!element || element.matches(':disabled'))
      throw Error(`Missing or disabled control: ${text}`);
    element.scrollIntoView({ block: 'nearest' });
    element.focus();
    element.click();
    await sleep();
  };
  const input = async (element, value) => {
    const prototype =
      element instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : element instanceof HTMLSelectElement
          ? HTMLSelectElement.prototype
          : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value').set.call(element, value);
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
    await sleep();
  };
  const resize = async (width, height) => {
    await send({ resize: true, width, height });
    await sleep();
  };
  const reset = async ({
    mode = 'normal',
    theme = 'light',
    collapsed = false,
    scenario = {},
  } = {}) => {
    window.__mode = mode;
    window.__scenario = scenario;
    window.__adopted = false;
    localStorage.setItem('omm.theme', theme);
    localStorage.setItem('omm.sidebar', collapsed ? 'collapsed' : 'expanded');
    window.__remount();
    await sleep(160);
  };
  const navigate = async (label) => {
    const link =
      document.querySelector(`nav [aria-label="${label}"]`) ||
      [...document.querySelectorAll('nav a')].find((element) =>
        element.textContent.includes(label),
      );
    if (!link) throw Error(`Missing route ${label}`);
    link.click();
    await sleep();
  };
  const detail = async () => {
    await navigate('会议库');
    const title = document.querySelector('.meeting-title');
    if (!title) throw Error('Missing meeting row');
    title.click();
    await sleep();
  };
  const dialog = () => [...document.querySelectorAll('[role="dialog"]')].at(-1);
  let records = 0;
  const measure = async (name, capture = true, extra = {}) => {
    const modal = dialog(),
      active = document.activeElement,
      root = modal || document.querySelector('main');
    const beforeFocus = {
      activeTag: active?.tagName,
      activeDisabled: active?.matches?.(':disabled') || false,
      focusInsideDialog: modal ? modal.contains(active) : undefined,
      documentFocused: document.hasFocus(),
    };
    const sidebar = document.querySelector('.sidebar');
    const scroll = {
      x: window.scrollX,
      y: window.scrollY,
      modal: modal?.scrollTop,
      sidebar: sidebar?.scrollTop,
    };
    const controls = [
      ...root.querySelectorAll('button,input,textarea,select,a[href],[role="slider"]'),
    ].filter(
      (element) =>
        visible(element) &&
        !element.matches(':disabled') &&
        element.getAttribute('tabindex') !== '-1',
    );
    const outside = [],
      obscuredControls = [];
    const shellControls = modal
      ? []
      : [
          ...document.querySelectorAll(
            '.sidebar button,.sidebar a[href],.topbar button,.topbar select',
          ),
        ].filter((element) => visible(element) && !element.matches(':disabled'));
    for (const element of [...controls, ...shellControls]) {
      if (element.closest('.table-scroll')) continue;
      element.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      const rect = element.getBoundingClientRect();
      if (rect.left < -1 || rect.right > innerWidth + 1)
        outside.push({
          text: element.textContent.slice(0, 50),
          left: rect.left,
          right: rect.right,
        });
      if (rect.top >= 0 && rect.bottom <= innerHeight && rect.width > 4 && rect.height > 4) {
        const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
        if (hit && hit !== element && !element.contains(hit) && !hit.contains(element))
          obscuredControls.push({
            text: element.getAttribute('aria-label') || element.textContent.slice(0, 50),
            hit: hit.className,
          });
      }
    }
    const last = controls.at(-1);
    let lastControlInView = true;
    if (last) {
      last.scrollIntoView({ block: 'nearest' });
      const rect = last.getBoundingClientRect();
      lastControlInView = rect.bottom <= innerHeight + 1 && rect.top >= -1;
    }
    window.scrollTo(scroll.x, scroll.y);
    if (modal) modal.scrollTop = scroll.modal;
    if (sidebar) sidebar.scrollTop = scroll.sidebar;
    const toast = document.getElementById('toast');
    const toastRect = toast?.getBoundingClientRect(),
      saveRect = document.querySelector('.settings-save')?.getBoundingClientRect();
    const result = {
      name,
      toastOpacity: toast ? getComputedStyle(toast).opacity : undefined,
      toastSaveOverlapPx:
        toastRect &&
        saveRect &&
        Math.min(toastRect.right, saveRect.right) > Math.max(toastRect.left, saveRect.left)
          ? Math.max(
              0,
              Math.min(toastRect.bottom, saveRect.bottom) - Math.max(toastRect.top, saveRect.top),
            )
          : 0,
      capture,
      width: innerWidth,
      height: innerHeight,
      scrollWidth: document.documentElement.scrollWidth,
      overflowElements:
        document.documentElement.scrollWidth > innerWidth + 1
          ? [...document.querySelectorAll('main *')]
              .filter((element) => {
                const rect = element.getBoundingClientRect();
                return rect.right > innerWidth + 1 && !element.closest('.table-scroll');
              })
              .slice(0, 18)
              .map((element) => ({
                tag: element.tagName,
                className: String(element.className),
                right: element.getBoundingClientRect().right,
                width: element.getBoundingClientRect().width,
                text: element.textContent.slice(0, 70),
              }))
          : [],
      theme: document.documentElement.dataset.mantineColorScheme,
      sidebar: document.documentElement.dataset.sidebar,
      outside,
      obscuredControls,
      lastControlInView,
      ...beforeFocus,
      dialogTitle:
        modal?.getAttribute('aria-labelledby') &&
        document.getElementById(modal.getAttribute('aria-labelledby'))?.textContent,
      modalScrollTop: modal?.scrollTop,
      controlCount: controls.length,
      playCalls: window.__playCalls,
      invalidResponses: window.__invalidResponses,
      cspViolations: window.__cspViolations,
      externalRequests: window.__externalRequests,
      tableWidths: [...root.querySelectorAll('.table-scroll')].map((element) => ({
        width: element.clientWidth,
        scrollWidth: element.scrollWidth,
      })),
      ...extra,
    };
    await send(result);
    records++;
  };
  const close = async () => {
    if (dialog()) await click('关闭对话框', dialog());
  };
  try {
    if (window.__runMode === '--smoke') {
      for (const [width, height, theme] of [
        [1280, 788, 'light'],
        [800, 568, 'dark'],
      ]) {
        await resize(width, height);
        await reset({ theme });
        await measure(`smoke-${theme}`);
      }
    } else {
      if (window.__runMode !== '--dialogs') {
        // All routes/detail tabs, themes, sidebar states and actual minimum content size.
        for (const [width, height] of [
          [1280, 788],
          [800, 568],
        ])
          for (const theme of ['light', 'dark'])
            for (const collapsed of [false, true]) {
              await resize(width, height);
              await reset({ theme, collapsed });
              const key = `${width}-${theme}-${collapsed ? 'collapsed' : 'expanded'}`;
              for (const label of labels) {
                await navigate(label);
                await measure(`${key}-${labels.indexOf(label)}`, !collapsed);
              }
              await detail();
              for (const tab of ['会议纪要', '逐字稿', '处理版本']) {
                await click(tab);
                await measure(`${key}-detail-${tab}`, !collapsed);
              }
            }
        await reset({ theme: 'system' });
        const systemTheme = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
        await measure('system-theme-current-host', true, {
          invalidTheme:
            document.documentElement.dataset.mantineColorScheme !== systemTheme ||
            localStorage.getItem('omm.theme') !== 'system',
          systemTheme,
        });
        for (const theme of ['dark', 'light']) {
          await input(document.querySelector('select[aria-label="外观主题"]'), theme);
          await measure(`theme-picker-${theme}`, true, {
            invalidTheme:
              document.documentElement.dataset.mantineColorScheme !== theme ||
              localStorage.getItem('omm.theme') !== theme,
          });
        }
        // Read-only/nullable preferences and genuinely pending read queries, not fake loading DOM.
        await resize(800, 568);
        await reset({
          mode: 'unknown',
          scenario: {
            readOnly: true,
            recordState: 'unknown',
            recordPrefs: { autoRecord: null, recordingConsent: null },
            configPrefs: { autoRecord: null, recordingConsent: null },
          },
        });
        await navigate('状态与诊断');
        const recordText = document.querySelector('.recording-bar').textContent;
        await measure('unknown-readonly-diagnostics', true, {
          invalidUnknown: !recordText.includes('录音状态未知') || recordText.includes('当前未录音'),
          recordText,
        });
        await navigate('设置');
        await measure('unknown-readonly-settings', true, {
          invalidUnknown: [...document.querySelectorAll('fieldset input,fieldset textarea')].some(
            (field) => !field.matches(':disabled'),
          ),
        });
        await reset({
          scenario: {
            pendingCommands: [
              'meeting_list',
              'task_list',
              'config_get',
              'diagnostics_get',
              'notification_history',
            ],
          },
        });
        for (const label of labels) {
          await navigate(label);
          await measure(`loading-route-${labels.indexOf(label)}`);
        }
        await reset({ scenario: { pendingCommands: ['meeting_get'] } });
        await detail();
        await measure('loading-meeting-detail');
        await reset({ scenario: { pendingCommands: ['artifact_read', 'artifact_versions'] } });
        await detail();
        for (const tab of ['会议纪要', '逐字稿', '处理版本']) {
          await click(tab);
          await measure(`loading-artifact-${tab}`);
        }
        // Empty, failed and unbroken long content for every route and all detail tabs.
        for (const mode of ['empty', 'error', 'long']) {
          await resize(800, 568);
          await reset({ mode, theme: mode === 'long' ? 'dark' : 'light' });
          for (const label of labels) {
            await navigate(label);
            await measure(`${mode}-route-${labels.indexOf(label)}`);
          }
          if (mode !== 'error') {
            if (mode === 'empty') {
              await reset({ scenario: { emptyArtifacts: true } });
              await detail();
            } else await detail();
            for (const tab of ['会议纪要', '逐字稿', '处理版本']) {
              await click(tab);
              await measure(`${mode}-detail-${tab}`);
            }
          }
        }
        // Every real settings group with long paths, both themes, min and roomy content.
        for (const [width, height, theme] of [
          [800, 568, 'light'],
          [1280, 788, 'dark'],
        ]) {
          await resize(width, height);
          await reset({ mode: 'long', theme });
          await navigate('设置');
          const groups = [...document.querySelectorAll('fieldset')];
          for (const [index, group] of groups.entries()) {
            group.scrollIntoView({ block: 'start' });
            await measure(`settings-${width}-${theme}-group-${index}`, true, {
              settingsGroup: group.querySelector('legend')?.textContent,
            });
          }
          const field = document.querySelector(
            'fieldset input:not([type="checkbox"]):not([inputmode="numeric"])',
          );
          await input(field, 'synthetic-config');
          await sleep(450);
          await click('配置生效时机');
          await measure(`settings-${width}-dirty-effects`);
          await click('放弃修改');
          await measure(`settings-${width}-discard`);
          await close();
          await navigate('会议库');
          await measure(`settings-${width}-leave-dirty`);
          await close();
        }
      }
      // CRUD/import, retention, reprocess and actual task/nested cancel dialogs.
      for (const theme of ['light', 'dark']) {
        await resize(800, 568);
        await reset({ mode: 'long', theme });
        await click('新建会议');
        await measure(`${theme}-create`);
        await click('选择音频文件');
        await measure(`${theme}-import-selected`);
        window.__failAction = true;
        await click('选择音频文件');
        await measure(`${theme}-create-error`, true, {
          missingDialogError: !dialog().querySelector('[role="alert"]'),
        });
        window.__failAction = false;
        await close();
        await detail();
        await click('编辑');
        await measure(`${theme}-edit`);
        window.__failAction = true;
        await click('保存');
        await measure(`${theme}-edit-error`, true, {
          missingDialogError: !dialog().querySelector('[role="alert"]'),
        });
        window.__failAction = false;
        await close();
        await click('重新处理');
        await measure(`${theme}-reprocess`);
        await close();
        await click('移入回收站');
        await measure(`${theme}-trash`);
        await close();
        await navigate('会议库');
        const visibility = document.querySelector('select option[value="trash"]')?.parentElement;
        if (visibility) {
          visibility.value = 'trash';
          visibility.dispatchEvent(new Event('change', { bubbles: true }));
          await sleep();
          document.querySelector('.table-scroll input[type=checkbox]').click();
          await sleep();
          await click('恢复');
          await measure(`${theme}-restore`);
          await close();
        }
        await navigate('处理队列');
        document.querySelector('.task-row .text-button').click();
        await sleep();
        await measure(`${theme}-task-detail`);
        await click('取消任务');
        await measure(`${theme}-nested-cancel`);
        await close();
        await close();
        await reset({
          theme,
          scenario: { recordPrefs: { autoRecord: false, recordingConsent: true } },
        });
        await click('启用自动录音');
        await measure(`${theme}-auto-confirm`);
        await close();
      }
      // Service unknown/loaded/adopt/stopped and asynchronous focus without measurement focusing.
      for (const theme of ['light', 'dark']) {
        await resize(800, 568);
        for (const mode of ['unknown', 'loaded', 'adopt']) {
          await reset({
            mode,
            theme,
            scenario:
              mode === 'unknown'
                ? {
                    recordState: 'unknown',
                    recordPrefs: { autoRecord: null, recordingConsent: null },
                    configPrefs: { autoRecord: null, recordingConsent: null },
                  }
                : mode === 'loaded'
                  ? {
                      recoveryPlan: {
                        state: 'unknown',
                        canApply: false,
                        planToken: null,
                        expiresAtMs: null,
                        reasons: ['external_unload_required'],
                        guidance:
                          '请使用原工具正常停止并解除已核归属旧登记；保留 plist、资料和配置。此处只展示指导文字，不执行命令。',
                      },
                    }
                  : {
                      adoption: true,
                      recoveryPlan: {
                        state: 'stopped',
                        reasons: ['profile_requires_takeover'],
                        effects: [
                          'Keep configuration, consent and database bytes unchanged; do not start a service',
                        ],
                        guidance: '仅采用登记，不启动；完成后另点启动。',
                      },
                    },
          });
          await navigate('状态与诊断');
          await measure(`${theme}-service-${mode}`);
          if (mode === 'adopt') {
            await click('安全切换服务版本');
            await measure(`${theme}-adopt-confirm`);
            await click('确认采用（不启动）');
            await measure(`${theme}-adopt-stopped`);
          }
        }
        await reset({ theme, scenario: { applyAllowed: true } });
        await navigate('状态与诊断');
        await click('启动后台服务');
        await measure(`${theme}-start-open`);
        window.__scenario = { applyAllowed: true, pendingCommands: ['service_apply'] };
        await click('确认启动');
        await measure(`${theme}-start-busy`);
        window.__pending.shift()();
        await sleep();
        await measure(`${theme}-start-failed`, true, {
          missingDialogError: !dialog().querySelector('[role="alert"]'),
        });
        window.__scenario = { applyAllowed: true, pendingCommands: ['service_plan'] };
        await click('重新检查计划');
        await measure(`${theme}-recheck-busy`);
        window.__pending.shift()();
        await sleep();
        await measure(`${theme}-recheck-failed`);
        window.__scenario = { applyAllowed: true };
        await click('重新检查计划');
        await measure(`${theme}-rechecked`);
        await close();
        window.__scenario = { applyAllowed: true, recoveryPlan: { expiresAtMs: Date.now() + 700 } };
        await click('启动后台服务');
        button('确认启动').focus();
        await sleep(1200);
        await measure(`${theme}-expired`, true, {
          confirmationDisabled: button('确认启动').disabled,
        });
        await close();
        window.__scenario = { applyAllowed: true, recoveryPlan: { state: 'running' } };
        await click('停止后台服务');
        await measure(`${theme}-stop-confirm`);
        await close();
        await click('安全切换服务版本');
        await measure(`${theme}-switch-confirm`);
        await close();
        await reset({ mode: 'empty', theme });
        await navigate('状态与诊断');
        await click('初始化本机资料库');
        await measure(`${theme}-initialize-confirm`);
        await close();
      }
      // Validation and failed save feedback must not obscure the form's actions.
      for (const [theme, count] of [
        ['light', 1],
        ['dark', 25],
      ]) {
        await resize(800, 568);
        await reset({ theme, scenario: { invalidConfig: true, invalidConfigCount: count } });
        await navigate('设置');
        await input(
          document.querySelector(
            'fieldset input:not([type="checkbox"]):not([inputmode="numeric"])',
          ),
          'synthetic-config',
        );
        await sleep(450);
        document.querySelector('.settings-save').scrollIntoView({ block: 'end' });
        await measure(`${theme}-settings-validation-${count}`);
        await reset({ theme });
        await navigate('设置');
        await input(
          document.querySelector(
            'fieldset input:not([type="checkbox"]):not([inputmode="numeric"])',
          ),
          'synthetic-config',
        );
        await sleep(450);
        window.__failAction = true;
        await click('保存配置');
        await measure(`${theme}-settings-save-error`);
        await click('关闭提示');
        await measure(`${theme}-settings-notice-dismissed`, false, {
          invalidNotice: !!document.getElementById('toast'),
        });
      }
      // Actual offline metadata/paused seek through the fixed synthetic WAV handler.
      // This is not the production archive/Range protocol; play is forbidden above.
      for (const theme of ['light', 'dark']) {
        await resize(800, 568);
        await reset({ theme, scenario: { mediaAvailable: true } });
        await detail();
        const audio = document.querySelector('audio');
        for (let attempt = 0; attempt < 20 && !Number.isFinite(audio.duration); attempt++)
          await sleep();
        await measure(`${theme}-player-metadata`, true, {
          invalidMedia:
            document.querySelectorAll('audio').length !== 1 ||
            !audio.paused ||
            audio.duration !== 20,
          mediaDuration: audio.duration,
          mediaPaused: audio.paused,
        });
        await click('前进 15 秒');
        await sleep();
        document.querySelector('.audio-player').scrollIntoView({ block: 'start' });
        await measure(`${theme}-player-paused-seek`, true, {
          invalidMedia: !audio.paused || audio.currentTime < 14.8,
          mediaSeekSeconds: audio.currentTime,
          mediaPaused: audio.paused,
        });
        await navigate('会议库');
        await measure(`${theme}-player-leave`, false, {
          invalidMedia:
            !audio.paused ||
            audio.getAttribute('src') !== null ||
            document.querySelectorAll('audio').length !== 0,
          releasedSrc: audio.getAttribute('src'),
          oldMediaPaused: audio.paused,
        });
      }
      // A genuine resize while a long document remains open.
      await reset({ mode: 'long', theme: 'dark', collapsed: true });
      await detail();
      await click('逐字稿');
      for (const [width, height] of [
        [1040, 700],
        [800, 568],
        [1280, 788],
      ]) {
        await resize(width, height);
        await measure(`resize-${width}`);
      }
    }
    await send({
      name: 'summary',
      combinationsCompleted: ['--smoke', '--dialogs'].includes(window.__runMode) ? 2 : 8,
      records,
    });
    await send({ done: true });
  } catch (error) {
    await send({ error: String(error), stack: error.stack });
    await send({ done: true });
  }
})();
