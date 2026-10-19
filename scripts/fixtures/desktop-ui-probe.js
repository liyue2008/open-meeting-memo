// A mutable task stage is not a record of its requested starting stage.
function reprocessingEvidence(stage, task, versions, transcript) {
  const version = versions.find(v => v.artifact.taskId === task.taskId && v.artifact.stage === 'transcribe' && v.status === 'committed');
  if (!version) return false;
  if (stage === 'summary') return task.stage === 'summary' && typeof version.sourceTranscriptTaskId === 'string';
  return stage === 'transcribe' && ['transcribe', 'summary'].includes(task.stage)
    && version.sourceTranscriptTaskId === null && transcript.includes('PRIVATE_SYNTHETIC_TRANSCRIPT');
}
// Real React events and independent production reads; never replace the frozen Tauri transport.
(async () => {
  const checks = [], reads = [], views = [], mediaEvidence = [], focusEvidence = [];
  let playCalls = 0, sequence = 0;
  // Optional fixture-only stage records: fixed numeric IDs, no requests or meeting data.
  const stageTraceStarted = performance.now();
  let stageTraceSequence = 0;
  const stageTrace = (stage, state) => {
    if (window.__ommFixtureStageTraceEnabled !== true || !Number.isInteger(stage) || stage < 0 || stage > 26 || !Number.isInteger(state) || state < 0 || state > 2) return;
    const queue = window.__ommFixtureStageTrace ??= [];
    if (queue.length >= 128) queue.shift();
    queue.push([stage, state, ++stageTraceSequence, Math.round(performance.now() - stageTraceStarted)]);
  };
  const invoke = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
  const check = (name, condition) => { if (!condition) throw Error(name); checks.push(name); };
  const raw = async (command, payload = {}) => {
    stageTrace(26, 0);
    const result = await invoke(command, { request: { schemaVersion: 1, requestId: `ui-driver-${++sequence}`, payload } });
    stageTrace(26, 1);
    reads.push({ command, error: result.error?.code ?? null });
    if (result.error) throw Error(command + ': ' + JSON.stringify(result.error));
    return result.data;
  };
  const tick = async () => { await raw('desktop_info'); await new Promise(r => setTimeout(r, 20)); };
  const visible = element => !!element && !element.closest('[hidden]') && element.getClientRects().length > 0;
  const elements = (selector, root = document) => [...root.querySelectorAll(selector)].filter(visible);
  const text = () => document.body.innerText;
  const wait = async (name, predicate) => {
    const deadline = Date.now() + 20000;
    do { const result = await predicate(); if (result) return result; await tick(); } while (Date.now() < deadline);
    throw Error('Timeout: ' + name);
  };
  const click = async (name, root = document) => {
    const button = await wait(name, () => elements('button,a,[role=tab]', root).find(e => !e.disabled && (e.textContent.trim() === name || e.getAttribute('aria-label') === name)));
    check('enabled control: ' + name, !button.disabled); button.scrollIntoView({block:'nearest'}); button.focus({preventScroll:true}); button.click(); await tick(); return button;
  };
  const label = (name, root = document) => {
    const wrapper = elements('label', root).find(e => e.childNodes[0]?.textContent.trim() === name);
    return wrapper?.querySelector('input,select,textarea') || (wrapper?.htmlFor && document.getElementById(wrapper.htmlFor)) || elements('[aria-label]', root).find(e => e.getAttribute('aria-label') === name);
  };
  const change = async (element, value) => {
    check('input exists: ' + value, !!element);
    element.scrollIntoView({block:'nearest'});
    if (element.getAttribute('role') === 'slider') {
      const track = element.closest('.mantine-Slider-root')?.querySelector('.mantine-Slider-trackContainer');
      check('actual Mantine mouse target', !!track);
      track.scrollIntoView({block:'center'});
      const rect = track.getBoundingClientRect();
      const minimum = Number(element.getAttribute('aria-valuemin'));
      const maximum = Number(element.getAttribute('aria-valuemax'));
      const fraction = (Number(value) - minimum) / (maximum - minimum);
      check('slider target within advertised bounds', fraction >= 0 && fraction <= 1 && rect.width > 0);
      const point = {bubbles:true, cancelable:true, clientX:rect.left + rect.width * fraction, clientY:rect.top + rect.height / 2, button:0};
      track.dispatchEvent(new MouseEvent('mousedown', {...point, buttons:1}));
      document.dispatchEvent(new MouseEvent('mouseup', {...point, buttons:0}));
      await tick(); return;
    }
    const proto = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(element, value);
    element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true })); await tick();
  };
  const modal = () => elements('dialog[open],[role=dialog]')[0];
  let titlebarInset;
  const shot = async request => {
    stageTrace(25, 0);
    window.__ommFixtureSnapshotResult = null; window.__ommFixtureSnapshotRequest = request;
    const result = await wait('owned WK snapshot ' + request.name, () => window.__ommFixtureSnapshotResult);
    check('owned content snapshot: ' + request.name, result.ok && result.name === request.name && result.bytes > 0);
    stageTrace(25, 1);
  };
  const snapshot = async (name, width = 1280, height = 820) => {
    if (titlebarInset === undefined) {
      await shot({name:'ui-viewport-calibration',width,height}); titlebarInset = height - innerHeight;
      check('measured actual UI titlebar inset', titlebarInset >= 0 && titlebarInset <= 100);
    }
    await shot({name:name + '-size',width,height:height + titlebarInset});
    check('actual UI content dimensions: ' + name, innerWidth === width && innerHeight === height);
    const active = modal();
    const last = active && elements('button',active).filter(e => !e.disabled).at(-1);
    last?.scrollIntoView({block:'nearest'}); await tick();
    await shot({name});
    const overflow = Math.max(0, document.documentElement.scrollWidth - innerWidth);
    check('no actual UI horizontal overflow: ' + name, overflow <= 1);
    views.push({ name, width: innerWidth, height: innerHeight, titlebarInset, overflow, theme: document.documentElement.dataset.mantineColorScheme, sidebar: document.documentElement.dataset.sidebar, body: text().slice(0, 12000) });
  };
  let advancedFilterObservation = null;
  // Independent two-frame witness, never a wait condition or a component callback.
  const advancedFrameHeartbeat = () => {
    const state = { callbacks: 0, status: 'not-called', cleanup: 'not-needed' };
    let active = true, pending = null, request, cancel;
    const schedule = callback => {
      try { pending = request(callback); state.status = 'scheduled'; }
      catch { state.status = 'schedule-failed'; }
    };
    const second = () => {
      pending = null;
      if (!active) return;
      state.callbacks = 2;
      state.status = 'complete';
    };
    const first = () => {
      pending = null;
      if (!active) return;
      state.callbacks = 1;
      schedule(second);
    };
    return {
      state,
      snapshot: () => ({ callbacks: state.callbacks, status: state.status }),
      start: () => {
        try {
          const requestFrame = window.requestAnimationFrame, cancelFrame = window.cancelAnimationFrame;
          if (typeof requestFrame !== 'function' || typeof cancelFrame !== 'function') { state.status = 'unavailable'; return; }
          request = callback => requestFrame.call(window, callback);
          cancel = id => cancelFrame.call(window, id);
          schedule(first);
        } catch { state.status = 'schedule-failed'; }
      },
      stop: () => {
        // Even if cancelling our handle fails, its late callback cannot schedule more work.
        active = false;
        if (pending === null) return;
        const id = pending;
        pending = null;
        try { cancel(id); state.cleanup = 'cancelled'; }
        catch { state.cleanup = 'failed'; }
      },
    };
  };
  const advancedFilterSnapshot = (phase, clickCalls, original, query, frames) => {
    const emptyControl = () => ({ exists: false, connected: false, visible: false, disabled: false, expanded: 'missing' });
    const emptyNode = () => ({ exists: false, connected: false, visible: false, rect: { x: 0, y: 0, width: 0, height: 0 }, display: 'unavailable', visibility: 'unavailable' });
    const unavailable = { phase, clickCalls, frames, read: 'unavailable', sameControl: false, original: emptyControl(), current: emptyControl(), controls: 'missing', labelledBy: 'missing', region: 'missing', panel: { ...emptyNode(), inlineDisplay: 'unavailable' }, sort: { ...emptyNode(), inPanel: false, disabled: false } };
    try {
      const { current, panel, sort } = query() ?? {};
      const bounded = (value, minimum = -16384) => Number.isFinite(value) ? Math.round(Math.min(16384, Math.max(minimum, value)) * 100) / 100 : 0;
      const controlState = node => {
        if (!node) return emptyControl();
        const expanded = node.getAttribute('aria-expanded');
        return { exists: true, connected: node.isConnected === true, visible: visible(node), disabled: node.matches(':disabled'), expanded: expanded === null ? 'missing' : ['true', 'false'].includes(expanded) ? expanded : 'other' };
      };
      const nodeState = node => {
        if (!node) return emptyNode();
        const rect = node.getBoundingClientRect(), style = getComputedStyle(node);
        const display = ['none', 'block', 'inline', 'inline-block', 'flex', 'inline-flex', 'grid', 'inline-grid', 'contents', 'table', 'list-item'].includes(style.display) ? style.display : 'other';
        const visibility = ['visible', 'hidden', 'collapse'].includes(style.visibility) ? style.visibility : 'other';
        return { exists: true, connected: node.isConnected === true, visible: visible(node), rect: { x: bounded(rect.x), y: bounded(rect.y), width: bounded(rect.width, 0), height: bounded(rect.height, 0) }, display, visibility };
      };
      const relation = (left, right) => !left || !right ? 'missing' : left === right ? 'match' : 'mismatch';
      const inline = panel?.style?.display;
      const inlineDisplay = typeof inline !== 'string' ? 'unavailable' : inline === '' ? 'empty' : ['none', 'block'].includes(inline) ? inline : 'other';
      return { phase, clickCalls, frames, read: 'ok', sameControl: !!original && original === current, original: controlState(original), current: controlState(current), controls: relation(current?.getAttribute('aria-controls'), panel?.id), labelledBy: relation(panel?.getAttribute('aria-labelledby'), current?.id), region: !panel ? 'missing' : panel.getAttribute('role') === 'region' ? 'valid' : 'invalid', panel: { ...nodeState(panel), inlineDisplay }, sort: { ...nodeState(sort), inPanel: !!panel && !!sort && panel.contains(sort), disabled: !!sort && sort.matches(':disabled') } };
    } catch {
      return unavailable;
    }
  };
  const openAdvanced = async () => {
    const findControl = () => elements('button[aria-expanded]').find(e => e.isConnected && !e.disabled && e.textContent.trim().startsWith('高级筛选与数据说明'));
    let original = null, lastQuery = null, clickCalls = 0;
    const frames = advancedFrameHeartbeat();
    advancedFilterObservation = { schemaVersion: 2, frames: frames.state, snapshots: [] };
    const record = (phase, query) => {
      // Diagnostics cannot change the original operation, timeout or thrown error.
      try {
        if (advancedFilterObservation.snapshots.length < 3) advancedFilterObservation.snapshots.push(advancedFilterSnapshot(phase, clickCalls, original, query, frames.snapshot()));
      } catch {}
    };
    const observationQuery = () => {
      const current = findControl();
      const panelId = current?.getAttribute('aria-controls');
      const panel = panelId && document.getElementById(panelId);
      const sort = panel && label('排序', panel);
      return { current, panel, sort };
    };
    try {
      const control = await wait('actual advanced Accordion control', findControl);
      original = control;
      record('before-activation', observationQuery);
      if (control.getAttribute('aria-expanded') !== 'true') { control.scrollIntoView({block:'nearest'}); clickCalls = 1; control.click(); }
      frames.start();
      record('after-activation', observationQuery);
      await wait('actual advanced Accordion expanded', () => {
        // React can replace a control after activation; never assert on its detached predecessor.
        const current = findControl();
        const panelId = current?.getAttribute('aria-controls');
        const panel = panelId && document.getElementById(panelId);
        const sort = panel && label('排序', panel);
        lastQuery = { current, panel, sort };
        return current?.getAttribute('aria-expanded') === 'true' && panel?.isConnected &&
          panel.getAttribute('role') === 'region' && panel.getAttribute('aria-labelledby') === current.id &&
          sort?.isConnected && panel.contains(sort) && visible(sort) && !sort.matches(':disabled') && sort;
      });
    } catch (error) {
      record('last-query', () => lastQuery);
      throw error;
    } finally {
      frames.stop();
    }
  };
  const modalCheckpoint = async name => {
    // Async file inspection changes enabled controls; test the settled modal's boundary set.
    const active = await wait('real modal initial focus ' + name, () => { const d = modal(); return d?.getAttribute('aria-busy') !== 'true' && d?.contains(document.activeElement) && !document.activeElement.matches(':disabled') && d; });
    const initial = document.activeElement;
    const targets = elements('button,input,select,textarea,a[href],[tabindex="0"]',active).filter(e => !e.disabled && e.tabIndex >= 0);
    check('real focusable modal controls ' + name, targets.length > 0);
    const first = targets[0], last = targets.at(-1);
    last.focus({preventScroll:true});
    last.dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',code:'Tab',keyCode:9,which:9,bubbles:true,cancelable:true})); await tick();
    check('actual forward Tab boundary ' + name, document.activeElement === first);
    first.dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',code:'Tab',keyCode:9,which:9,shiftKey:true,bubbles:true,cancelable:true})); await tick();
    check('actual reverse Tab boundary ' + name, document.activeElement === last);
    last.scrollIntoView({block:'nearest'});
    const rect = last.getBoundingClientRect(), hit = document.elementFromPoint(rect.left + rect.width/2,rect.top + rect.height/2);
    check('last enabled modal control reachable ' + name, rect.top >= 0 && rect.bottom <= innerHeight + 1 && !!hit && (hit === last || last.contains(hit)));
    focusEvidence.push({name,initialTag:initial.tagName,initialDisabled:initial.disabled ?? false,focusable:targets.length,forwardWrap:true,reverseWrap:true,lastReachable:true,syntheticKeyboardEvents:true});
    if (initial.isConnected) initial.focus({preventScroll:true});
  };
  const localRange = preset => {
    if (preset === 'all') return null;
    const now = new Date(), start = new Date(now.getFullYear(), now.getMonth(), now.getDate()), end = new Date(start);
    end.setDate(end.getDate() + 1);
    if (preset === 'week') start.setDate(start.getDate() - ((start.getDay() + 6) % 7));
    if (preset === '3' || preset === '7') start.setDate(start.getDate() - Number(preset) + 1);
    const day = d => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
    return { fromLocalDate: day(start), toLocalDateExclusive: day(end), timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone, fromUtcMs: start.getTime(), toUtcMsExclusive: end.getTime() };
  };
  const list = (query = '', date = null, sort = 'started_desc') => raw('meeting_list', { filter: { query, scope: 'title', date, sources: [], statuses: [], sort, visibility: 'active' }, cursor: null, pageSize: 25, expectedRevision: null });
  const tasks = (meetingId = null, statuses = []) => raw('task_list', { meetingId, statuses, cursor: null, pageSize: 100 });
  const titles = () => elements('.meeting-title').map(e => e.textContent);
  try {
    stageTrace(0, 0);
    stageTrace(1, 0);
    const forbidPlay = function () { playCalls++; throw Error('Audio playback forbidden'); };
    HTMLMediaElement.prototype.play = forbidPlay;
    check('playback guard installed before media interaction', HTMLMediaElement.prototype.play === forbidPlay);
    check('actual trusted WKWebView origin', location.origin === 'tauri://localhost');
    const info = await wait('private service ownership', async () => { const i = await raw('desktop_info'); return i.compatibility.writeAdmission?.mode === 'service' && i; });
    const config = await raw('config_get');
    check('owned fake worker and Pi only; automatic recording off', config.values.workerExecutable.endsWith('/fake-worker') && config.values.piExecutable.endsWith('/fake-pi') && !config.values.autoRecord);
    stageTrace(1, 1);
    stageTrace(2, 0);
    await wait('production React real library', () => titles().length);
    check('React has no mount or validator error', elements('[role=alert]').length === 0);
    check('real app version rendered', text().includes('v' + info.appVersion));
    check('default seven local days', label('日期').value === '7');
    check('real server first page has 25 meetings', elements('tbody tr').length === 25);
    stageTrace(2, 1);
    stageTrace(3, 0);
    await click('下一页');
    await wait('server second page', () => text().includes('第 2 页') && elements('tbody tr').length > 0);
    check('real second page populated', elements('tbody tr').length > 0);
    await click('上一页');
    stageTrace(3, 1);
    stageTrace(4, 0);
    check('advanced filters initially collapsed', elements('button[aria-expanded]').find(e => e.textContent.trim().startsWith('高级筛选与数据说明'))?.getAttribute('aria-expanded') === 'false');
    await openAdvanced();
    for (const preset of ['today', 'week', '3', '7', 'all']) {
      await change(label('日期'), preset);
      const sort = { today: 'started_asc', week: 'duration_desc', '3': 'duration_asc', '7': 'started_asc', all: 'started_desc' }[preset];
      await change(label('排序'), sort);
      const expected = await list('', localRange(preset), sort);
      await wait('React local-date/sort matches independent server results: ' + preset, () => JSON.stringify(titles()) === JSON.stringify(expected.items.map(m => m.metadata.title)));
      check('selected local-day preset and server ordering: ' + preset, label('日期').value === preset && label('排序').value === sort);
    }
    stageTrace(4, 1);
    stageTrace(5, 0);
    await change(label('搜索范围'), 'content');
    await change(label('搜索会议'), 'Private synthetic transcript token');
    const contentResults = await raw('meeting_list', { filter: { query: 'Private synthetic transcript token', scope: 'content', date: localRange(label('日期').value), sources: [], statuses: [], sort: label('排序').value, visibility: 'active' }, cursor: null, pageSize: 25, expectedRevision: null });
    check('completed synthetic transcripts are actually searchable', contentResults.items.some(m => m.metadata.title === 'Private bridge fixture'));
    await wait('indexed transcript search matches independent server results', () => JSON.stringify(titles()) === JSON.stringify(contentResults.items.map(m => m.metadata.title)));
    await change(label('搜索范围'), 'title');
    await change(label('搜索会议'), 'Private bridge fixture');
    await wait('debounced server title search', () => titles().length === 1 && titles()[0] === 'Private bridge fixture');
    stageTrace(5, 1);
    stageTrace(6, 0);
    label('选择 Private bridge fixture').click(); await tick();
    await click('批量导出纪要');
    await wait('restricted native batch export acknowledged', () => text().includes('纪要已导出'));
    stageTrace(6, 1);
    await click('取消选择');
    stageTrace(7, 0);
    await click('Private bridge fixture');
    await wait('actual summary content', () => elements('.markdown').some(e => e.textContent.includes('Synthetic summary')));
    const player = document.querySelector('.audio-player'), artifacts = document.querySelector('.artifact-panel');
    check('recording above transcript and minutes', !!(player.compareDocumentPosition(artifacts) & Node.DOCUMENT_POSITION_FOLLOWING));
    check('real synthetic actions visible and read only', document.querySelector('.action-items')?.textContent.includes('Read-only synthetic action') && elements('.action-items li').length > 0 && !document.querySelector('.action-items input[type=checkbox]'));
    check('summary checkbox source has no interactive checkbox', !document.querySelector('.artifact-panel input[type=checkbox]'));
    stageTrace(7, 1);
    stageTrace(8, 0);
    const mediaMeeting = (await list('Private bridge fixture')).items[0];
    const manifest = await raw('media_manifest', { meetingId: mediaMeeting.meetingId, cursor: null, pageSize: 100 });
    const grant = await raw('media_open', { meetingId: mediaMeeting.meetingId, segmentId: manifest.segments[0].segmentId, expectedRevision: manifest.revision });
    mediaEvidence.push({ manifestRevision: manifest.revision, grant });
    await raw('media_close', { grantId: grant.grantId });
    check('native media grant revision matches requested manifest revision', grant.revision === manifest.revision);
    stageTrace(8, 1);
    stageTrace(9, 0);
    let audio = await wait('player metadata', () => { const a = document.querySelector('audio'); return a?.readyState >= 1 && a; });
    check('player paused metadata no playback', audio.paused && Number.isFinite(audio.duration));
    stageTrace(9, 1);
    stageTrace(10, 0);
    await change(label('会议时间定位'), '1200');
    await wait('real paused intra-segment seek', () => audio.paused && Math.abs(audio.currentTime - 1.2) < 0.1);
    mediaEvidence.push({ kind: 'intra-segment', paused: audio.paused, currentTime: audio.currentTime });
    const segments = label('录音分段');
    check('real two-segment media manifest', segments.options.length === 2);
    stageTrace(10, 1);
    stageTrace(11, 0);
    const firstSource = audio.currentSrc;
    await change(segments, segments.options[1].value);
    audio = await wait('paused cross-segment seek', () => { const a = document.querySelector('audio'); return a?.readyState >= 1 && a.currentSrc !== firstSource && a; });
    check('cross segment remains paused', audio.paused);
    stageTrace(11, 1);
    stageTrace(12, 0);
    await change(label('会议时间定位'), '4500');
    await wait('real paused second-segment offset seek', () => audio.paused && Math.abs(audio.currentTime - 1.5) < 0.1);
    mediaEvidence.push({ kind: 'cross-segment', paused: audio.paused, currentTime: audio.currentTime, selectedSegment: segments.value });
    stageTrace(12, 1);
    stageTrace(13, 0);
    await change(label('速度'), '1.25');
    check('paused speed control changes actual media element', audio.paused && audio.playbackRate === 1.25);
    stageTrace(13, 1);
    stageTrace(14, 0);
    await snapshot('detail-audio-top');
    stageTrace(14, 1);
    stageTrace(15, 0);
    await click('复制完整内容'); await wait('native clipboard acknowledged in UI', () => text().includes('已复制完整产物'));
    await click('导出'); await wait('restricted export acknowledged in UI', () => text().includes('内容已导出'));
    await click('逐字稿');
    await wait('real transcript rendered', () => text().includes('Private synthetic transcript token'));
    await change(label('当前已载入逐字稿内搜索'), 'synthetic');
    check('transcript search keeps matching line', elements('.transcript-line').length > 0);
    const timestamp = await wait('enabled transcript timestamp', () => elements('.transcript-line button').find(e => !e.disabled));
    timestamp.click(); await tick();
    await wait('timestamp locates first segment paused', () => label('录音分段').selectedIndex === 0 && document.querySelector('audio')?.readyState >= 1 && document.querySelector('audio').paused);
    await click('处理版本'); await wait('real versions', () => elements('.version-row').length >= 2);
    stageTrace(15, 1);
    stageTrace(16, 0);
    await click('编辑'); await modalCheckpoint('metadata-edit'); await change(label('会议标题'), 'UI edited synthetic title'); await click('保存', modal());
    await wait('metadata CAS updates DOM', () => !modal() && text().includes('UI edited synthetic title'));
    check('metadata persisted by real use case', (await list('UI edited synthetic title')).items.length === 1);
    await click('移入回收站'); await modalCheckpoint('retained-trash'); await click('移入回收站', modal());
    await wait('retained trash detail', () => !modal() && text().includes('此会议位于回收站'));
    await click('恢复会议'); await modalCheckpoint('retained-restore'); await click('确认恢复', modal());
    await wait('restored detail', () => !modal() && !text().includes('此会议位于回收站'));
    const editedMeeting = (await list('UI edited synthetic title')).items[0];
    stageTrace(16, 1);
    stageTrace(17, 0);
    for (const stage of ['summary', 'transcribe']) {
      const priorTasks = (await tasks(editedMeeting.meetingId)).items.map(t => t.taskId);
      await click('重新处理'); await modalCheckpoint('reprocess-' + stage); await change(label('处理范围'), stage); await click('开始处理 1 场', modal());
      await wait('explicit per-meeting queued outcome', () => modal()?.textContent.includes('任务已入队') || modal()?.textContent.includes('后台操作'));
      await click('关闭', modal());
      const enqueued = await wait('new retained ' + stage + ' generation', async () => (await tasks(editedMeeting.meetingId)).items.find(t => !priorTasks.includes(t.taskId)));
      const executed = await wait('real reprocessing execution evidence: ' + stage, async () => {
        const current = await raw('task_get', { taskId: enqueued.taskId });
        if (!current.versions.some(v => v.artifact.taskId === enqueued.taskId && v.artifact.stage === 'transcribe' && v.status === 'committed')) return false;
        const artifact = stage === 'transcribe' ? await raw('artifact_read', { artifact: { meetingId: editedMeeting.meetingId, taskId: enqueued.taskId, stage: 'transcribe' }, cursor: null, maxBytes: 4096 }) : null;
        const transcript = artifact?.text ?? '';
        return reprocessingEvidence(stage, current.task, current.versions, transcript) ? { ...current, transcript } : false;
      });
      check('real reprocessing requested work: ' + stage, reprocessingEvidence(stage, executed.task, executed.versions, executed.transcript));
      await click('取消任务'); await modalCheckpoint('cancel-' + stage); await click('确认取消', modal());
      await wait('owned reprocessing cancellation settled', async () => (await raw('task_get', { taskId: enqueued.taskId })).task.status === 'cancelled');
      // App queries stay fresh for 1000 ms and pause background polling. Reopen only once that real cache is stale.
      const cancellationSettledAt = Date.now();
      await wait('detail cache stale before genuine remount', () => Date.now() - cancellationSettledAt > 1100);
      // Re-enter through user navigation after the owned cancellation settles; this also exercises retention.
      await click('会议库'); await wait('library after cancellation', () => label('搜索会议'));
      await change(label('搜索会议'), 'UI edited synthetic title');
      await wait('cancelled meeting remains searchable', () => titles().length === 1 && titles()[0] === 'UI edited synthetic title');
      await click('UI edited synthetic title');
      await wait('reopened detail has real settled cancellation', () => elements('.task-row').some(row => row.textContent.includes('已取消')));
    }
    stageTrace(17, 1);
    stageTrace(18, 0);
    await click('会议库'); await wait('library after restore', () => label('日期'));
    await click('＋ 新建会议'); await modalCheckpoint('new-import'); await change(label('会议标题'), 'UI synthetic imported meeting'); await click('选择音频文件', modal());
    await wait('restricted native file selection', () => label('原会议开始时间（本地）'));
    await modalCheckpoint('selected-import');
    const now = new Date(), two = value => String(value).padStart(2, '0');
    await change(label('原会议开始时间（本地）'), `${now.getFullYear()}-${two(now.getMonth() + 1)}-${two(now.getDate())}T${two(now.getHours())}:${two(now.getMinutes())}`);
    await click('导入', modal());
    await wait('real import persisted', async () => !modal() && (await list('UI synthetic imported meeting')).items.length === 1);
    stageTrace(18, 1);
    stageTrace(19, 0);
    await click('＋ 新建会议'); await modalCheckpoint('new-fake-recording'); await change(label('会议标题'), 'UI fake-helper recording');
    check('recording requires independent explicit consent', elements('button', modal()).find(e => e.textContent.trim() === '开始录音').disabled);
    elements('input[type=checkbox]', modal())[0].click(); await click('开始录音', modal());
    await wait('fake-helper recording', () => !modal() && text().includes('正在录音'));
    await click('停止并封存');
    await wait('fake-helper recording sealed', async () => text().includes('当前未录音') && (await raw('record_status')).state === 'idle');
    stageTrace(19, 1);
    stageTrace(20, 0);
    await click('设置'); await wait('config loaded', () => label('Pi 模型'));
    const base = await raw('config_get');
    await change(label('Pi 模型'), 'ui-private-draft'); await click('使用帮助');
    await wait('dirty navigation guard', () => modal()?.textContent.includes('设置尚未保存'));
    await modalCheckpoint('dirty-navigation');
    await click('留在设置', modal()); check('cancel navigation preserves draft', label('Pi 模型').value === 'ui-private-draft');
    const current = await raw('desktop_info');
    await raw('config_save', { admission: current.compatibility.writeAdmission, expectedRevision: base.revision, patch: { startDelayMs: base.values.startDelayMs + 1 } });
    await click('保存配置'); await wait('actual CAS conflict in UI', () => elements('[role=alert]').length > 0);
    check('config conflict retains draft', label('Pi 模型').value === 'ui-private-draft');
    check('conflicting UI write did not replace concurrent config', (await raw('config_get')).values.piModel === base.values.piModel);
    await click('使用帮助'); await click('放弃并离开', modal());
    await wait('real help', () => !modal() && text().includes('使用帮助'));
    await click('设置'); await wait('config after discard', () => label('自动录音开始延迟（毫秒）'));
    await click('重新读取配置'); await modalCheckpoint('reload-config'); await click('放弃草稿并读取', modal());
    await wait('explicit config refresh after conflict', () => label('自动录音开始延迟（毫秒）').value === String(base.values.startDelayMs + 1));
    await change(label('自动录音开始延迟（毫秒）'), String(base.values.startDelayMs + 2)); await click('保存配置');
    await wait('successful UI config CAS save', () => text().includes('当前没有未保存的修改'));
    const saved = await raw('config_get');
    check('atomic UI save persisted value and revision', saved.values.startDelayMs === base.values.startDelayMs + 2 && saved.revision !== base.revision);
    stageTrace(20, 1);
    stageTrace(21, 0);
    await click('状态与诊断'); await wait('real diagnostics rendered', () => text().includes('源码版本'));
    await snapshot('diagnostics');
    await click('处理队列'); await wait('queue loaded', () => label('任务状态'));
    await change(label('任务状态'), 'failed');
    const failed = (await tasks(null, ['failed'])).items[0]; check('finite failed fixture exists', !!failed?.canRetry);
    await click('重试失败阶段');
    await wait('real failed task retry persisted', async () => { const t = (await raw('task_get', { taskId: failed.taskId })).task; return t.status !== 'failed' || t.retryCount > failed.retryCount; });
    await change(label('任务状态'), 'cancelled');
    const cancelled = (await tasks(null, ['cancelled'])).items[0]; check('cancelled fixture exists', !!cancelled);
    const before = (await tasks(cancelled.meetingId)).items.map(t => t.taskId);
    await click('重新入队');
    await wait('explicit cancelled recovery creates new task', async () => (await tasks(cancelled.meetingId)).items.some(t => !before.includes(t.taskId)));
    stageTrace(21, 1);
    stageTrace(22, 0);
    await click('会议库'); await wait('library for appearance', () => label('日期'));
    for (const theme of ['light', 'dark']) {
      await change(label('外观主题'), theme); await wait(theme + ' theme', () => document.documentElement.dataset.theme === theme);
      check(theme + ' private-window preference stored', localStorage.getItem('omm.theme') === theme);
      await snapshot('library-' + theme + '-expanded');
    }
    await click('收起侧栏'); check('sidebar stored collapsed in private window', localStorage.getItem('omm.sidebar') === 'collapsed');
    await snapshot('library-dark-collapsed');
    await snapshot('library-dark-minimum', 800, 600);
    check('minimum owned content no root horizontal overflow', innerWidth === 800 && innerHeight === 600 && document.documentElement.scrollWidth <= innerWidth + 1);
    await change(label('外观主题'), 'light'); await snapshot('library-light-narrow');
    await click('展开侧栏'); await change(label('外观主题'), 'system');
    check('system mode matches actual WK preference', document.documentElement.dataset.theme === (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'));
    const keys = Array.from({ length: localStorage.length }, (_, i) => localStorage.key(i));
    check('only appearance in private web storage', keys.every(k => ['omm.theme', 'omm.sidebar'].includes(k)));
    check('no audio playback', playCalls === 0);
    await click('设置');
    stageTrace(22, 1);
    stageTrace(23, 0);
    const closeField = await wait('editable config before native close', () => { const field = label('自动录音开始延迟（毫秒）'); return field && !field.disabled && field; });
    const closeBaseline = await raw('config_get');
    const closeDraft = String(closeBaseline.values.startDelayMs + 123);
    await change(closeField, closeDraft);
    await wait('actual dirty settings before native close', () => text().includes('有未保存的修改'));
    const requestClose = async request => {
      window.__ommFixtureCloseResult = null;
      window.__ommFixtureCloseRequest = request;
      const receipt = await wait('owned native close receipt: ' + request.id, () => window.__ommFixtureCloseResult);
      check('trusted matching native close receipt: ' + request.id, receipt.ok === true && receipt.id === request.id);
      return receipt;
    };
    const cancelledClose = await requestClose({ id: 'ui-close-cancel', action: 'close' });
    check('first actual Rust close requested', cancelledClose.nativeCloseRequested === true);
    await wait('actual React dirty close confirmation', () => modal()?.textContent.includes('设置尚未保存'));
    await modalCheckpoint('native-close-cancel');
    await click('留在设置', modal());
    await wait('cancel keeps real window and unsaved draft', () => !modal() && closeField.isConnected && closeField.value === closeDraft && text().includes('有未保存的修改'));
    const afterCancel = await raw('config_get');
    check('cancelled native close did not save disk configuration', afterCancel.revision === closeBaseline.revision && JSON.stringify(afterCancel.values) === JSON.stringify(closeBaseline.values));
    const confirmedClose = await requestClose({ id: 'ui-close-confirm', action: 'close' });
    check('second actual Rust close requested', confirmedClose.nativeCloseRequested === true);
    await wait('second actual React close confirmation', () => modal()?.textContent.includes('设置尚未保存'));
    await modalCheckpoint('native-close-confirm');
    check('draft remains unsaved until actual discard', closeField.isConnected && closeField.value === closeDraft && (await raw('config_get')).revision === closeBaseline.revision);
    await snapshot('dirty-close-confirmation');
    const discard = await wait('enabled real React discard before arming', () => elements('button', modal()).find(button => !button.disabled && button.textContent.trim() === '放弃并离开'));
    check('enabled actual React discard button', !!discard);
    // The native arm channel is bounded to 64 KiB; retain assertions and aggregate diagnostic polling.
    const readCounts = reads.reduce((counts, read) => { const key = read.command + ':' + (read.error ?? 'ok'); counts[key] = (counts[key] ?? 0) + 1; return counts; }, {});
    const closeGuard = { cancelledWindowAndDraftRetained: true, cancelledDiskUnchanged: true, baselineRevision: closeBaseline.revision, draft: closeDraft };
    check('fourteen actual modal focus checkpoints completed', focusEvidence.length === 14);
    stageTrace(23, 1);
    stageTrace(24, 0);
    const armed = await requestClose({ id: 'ui-close-finish', action: 'arm-destroy', result: { ok: true, checks, playCalls, views: views.map(({ body: _body, ...view }) => view), focusEvidence, mediaEvidence, independentNativeReadCounts: readCounts, closeGuard, storage: 'nonpersistent isolated fixture; no cross-process persistence claim', userAgent: navigator.userAgent } });
    check('native evidence armed before actual React discard', armed.armed === true && armed.mainAlive === true);
    check('real discard still connected after native arming', discard.isConnected && !discard.disabled);
    // No IPC or generic completion after this click: native destruction and owned cleanup decide success.
    stageTrace(24, 1);
    stageTrace(0, 1);
    discard.click();
  } catch (error) {
    stageTrace(0, 2);
    window.__ommFixtureResult = { ok: false, checks, playCalls, error: String(error), views, focusEvidence, mediaEvidence, visibility: document.visibilityState, focused: document.hasFocus(), body: text().slice(0, 16000), independentNativeReads: reads, advancedFilterObservation };
  }
})();
