(async () => {
  const checks = [];
  let playCalls = 0;
  HTMLMediaElement.prototype.play = function () { playCalls++; return Promise.reject(new Error('playback forbidden')); };
  const text = () => document.body.innerText;
  const modal = () => document.querySelector('dialog[open],[role=dialog]');
  const elements = (selector, root = document) => [...root.querySelectorAll(selector)];
  const check = (name, value) => { if (!value) throw new Error(name); checks.push(name); };
  const raw = async (command, payload) => {
    const reply = await window.__TAURI_INTERNALS__.invoke(command, { request: { schemaVersion: 1, requestId: crypto.randomUUID(), payload } });
    if (reply.error) throw new Error(command + ': ' + JSON.stringify(reply.error));
    return reply.data;
  };
  const wait = async (name, fn) => {
    const deadline = Date.now() + 20000;
    while (Date.now() < deadline) {
      const value = await fn();
      if (value) return value;
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    throw new Error('Timeout: ' + name);
  };
  const label = name => elements('label').find(e => e.childNodes[0]?.textContent.trim() === name)?.querySelector('input,select,textarea') || document.querySelector('[aria-label="' + name + '"]');
  const button = (name, root = document) => elements('button,a', root).find(e => (e.getAttribute('aria-label') || e.textContent).trim() === name);
  const click = async (name, root) => { const e = await wait('enabled ' + name, () => { const e = button(name, root); return e && !e.disabled && e.getClientRects().length && e; }); e.click(); };
  const change = (element, value) => {
    const proto = element.tagName === 'SELECT' ? HTMLSelectElement.prototype : element.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(element, value);
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const titles = () => elements('.meeting-title').map(e => e.textContent.trim());
  const title = 'F5onlyTitle synthetic retained meeting';
  let meetingId, taskId;
  try {
    await wait('real React library', () => label('搜索会议'));
    change(label('日期'), 'all');
    change(label('搜索会议'), 'UI failed task fixture');
    await wait('seeded failed meeting', () => titles().length === 1 && titles()[0] === 'UI failed task fixture');
    const found = await raw('meeting_list', { filter: { query: 'UI failed task fixture', scope: 'title', date: null, sources: [], statuses: [], sort: 'started_desc', visibility: 'active' }, cursor: null, pageSize: 25, expectedRevision: null });
    check('real seeded failed meeting found', found.items.length === 1);
    meetingId = found.items[0].meetingId;
    const tasks = await raw('task_list', { meetingId, statuses: [], cursor: null, pageSize: 25 });
    check('failed fixture has no successful content to match', tasks.items.length === 1 && tasks.items[0].status === 'failed');
    taskId = tasks.items[0].taskId;
    await click('UI failed task fixture');
    await click('编辑'); await wait('edit dialog', () => modal());
    change(label('会议标题'), title);
    await click('保存', modal());
    await wait('real metadata CAS saved', () => !modal() && text().includes(title));
    check('metadata-only unique title saved through UI', true);
    await click('会议库'); await wait('library content controls', () => label('搜索范围'));
    change(label('日期'), 'all'); change(label('搜索范围'), 'content'); change(label('搜索会议'), title);
    await wait('content scope matches only metadata title', () => titles().length === 1 && titles()[0] === title);
    const content = await raw('meeting_list', { filter: { query: title, scope: 'content', date: null, sources: [], statuses: [], sort: 'started_desc', visibility: 'active' }, cursor: null, pageSize: 25, expectedRevision: null });
    check('actual content scope includes title without successful artifacts', content.items.length === 1 && content.items[0].meetingId === meetingId);
    check('React content search renders native title-only match', label('搜索范围').value === 'content' && titles()[0] === title);
    await click(title); await click('移入回收站'); await click('移入回收站', modal());
    await wait('real Trash state committed', () => !modal() && text().includes('此会议位于回收站'));
    check('Trash retains failed meeting', true);
    await click('处理队列');
    let row = await wait('actual failed task row', () => elements('.task-row').find(e => e.textContent.includes(meetingId)));
    await wait('Trash disables retry and requeue', () => button('重试任务', row)?.disabled && button('重新入队', row)?.disabled);
    const blocked = await raw('task_get', { taskId });
    check('native task capabilities deny Trash retry and requeue', !blocked.task.canRetry && !blocked.task.canRequeue && blocked.task.status === 'failed');
    check('actual React queue disables Trash mutations', button('重试任务', row).disabled && button('重新入队', row).disabled);
    await click('会议库'); await wait('Trash library controls', () => label('位置'));
    change(label('日期'), 'all'); change(label('位置'), 'trash'); change(label('搜索会议'), title);
    await wait('retained Trash meeting searchable', () => titles().length === 1 && titles()[0] === title);
    await click(title); await click('恢复会议'); await click('确认恢复', modal());
    await wait('restore committed', () => !modal() && !text().includes('此会议位于回收站'));
    await click('处理队列');
    row = await wait('restored task row', () => elements('.task-row').find(e => e.textContent.includes(meetingId)));
    await wait('restore returns real queue capabilities', () => !button('重试任务', row)?.disabled && !button('重新入队', row)?.disabled);
    const restored = await raw('task_get', { taskId });
    check('restore preserves task identity and restores eligible capabilities', restored.task.taskId === taskId && restored.task.canRetry && restored.task.canRequeue);
    check('actual React queue reflects restored capabilities', !button('重试任务', row).disabled && !button('重新入队', row).disabled);
    check('no playback requested', playCalls === 0);
    window.__ommFixtureResult = { ok: true, checks, playCalls, meetingId, taskId, scope: 'F2 Trash UI and F5 title-only content; active-attempt/late-shutdown races remain Core regression evidence' };
  } catch (error) {
    window.__ommFixtureResult = { ok: false, checks, playCalls, error: String(error), meetingId, taskId, body: text().slice(0, 12000) };
  }
})();
