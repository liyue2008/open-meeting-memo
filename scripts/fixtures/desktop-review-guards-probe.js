(async () => {
  const checks = [];
  let playCalls = 0;
  HTMLMediaElement.prototype.play = function () { playCalls++; return Promise.reject(new Error('playback forbidden')); };
  const text = () => document.body.innerText;
  const modal = () => document.querySelector('dialog[open],[role=dialog]');
  const elements = (selector, root = document) => [...root.querySelectorAll(selector)].filter(e => !e.closest('[hidden]') && e.getClientRects().length > 0);
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
  const button = (name, root = document) => elements('button,a', root).find(e => e.textContent.trim() === name || e.getAttribute('aria-label') === name);
  const click = async (name, root) => { const e = await wait('enabled ' + name, () => { const e = button(name, root); return e && !e.disabled && e.getClientRects().length && e; }); e.click(); };
  const change = (element, value) => {
    const proto = element.tagName === 'SELECT' ? HTMLSelectElement.prototype : element.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(element, value);
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const titles = () => elements('.meeting-title').map(e => e.textContent.trim());
  const cases = [
    { title: 'UI failed task fixture', status: 'failed', retry: true, requeue: false },
    { title: 'UI cancelled task fixture', status: 'cancelled', retry: false, requeue: true },
  ];
  const evidence = [];
  let meetingId, taskId;
  try {
    for (const fixture of cases) {
      const title = 'F5onlyTitle synthetic retained ' + fixture.status;
      await click('会议库'); await wait('real React library', () => label('搜索会议'));
      change(label('日期'), 'all'); change(label('位置'), 'active'); change(label('搜索范围'), 'title');
      change(label('搜索会议'), fixture.title);
      await wait('seeded settled meeting', () => titles().length === 1 && titles()[0] === fixture.title);
      const found = await raw('meeting_list', { filter: { query: fixture.title, scope: 'title', date: null, sources: [], statuses: [], sort: 'started_desc', visibility: 'active' }, cursor: null, pageSize: 25, expectedRevision: null });
      check(fixture.status + ': real seeded meeting found', found.items.length === 1);
      meetingId = found.items[0].meetingId;
      const tasks = await raw('task_list', { meetingId, statuses: [], cursor: null, pageSize: 25 });
      check(fixture.status + ': no successful content to match', tasks.items.length === 1 && tasks.items[0].status === fixture.status);
      taskId = tasks.items[0].taskId;
      check(fixture.status + ': expected eligible initial capabilities', tasks.items[0].canRetry === fixture.retry && tasks.items[0].canRequeue === fixture.requeue);
      await click(fixture.title);
      await wait('actual settled detail', () => elements('h1').some(e => e.textContent === fixture.title) && text().includes('本场处理任务'));
      await click('编辑'); await wait('edit dialog', () => modal()); change(label('会议标题'), title); await click('保存', modal());
      await wait('real metadata CAS saved', () => !modal() && text().includes(title));
      check(fixture.status + ': metadata-only unique title saved through UI', true);
      await click('会议库'); await wait('library content controls', () => label('搜索范围'));
      change(label('日期'), 'all'); change(label('搜索范围'), 'content'); change(label('搜索会议'), title);
      await wait('content scope matches only metadata title', () => titles().length === 1 && titles()[0] === title);
      const content = await raw('meeting_list', { filter: { query: title, scope: 'content', date: null, sources: [], statuses: [], sort: 'started_desc', visibility: 'active' }, cursor: null, pageSize: 25, expectedRevision: null });
      check(fixture.status + ': actual content scope includes title without successful artifacts', content.items.length === 1 && content.items[0].meetingId === meetingId);
      check(fixture.status + ': React content search renders native title-only match', label('搜索范围').value === 'content' && titles()[0] === title);
      await click(title); await click('移入回收站'); await click('移入回收站', modal());
      await wait('real Trash state committed', () => !modal() && text().includes('此会议位于回收站'));
      check(fixture.status + ': Trash retains settled meeting', true);
      await click('处理队列');
      const row = () => elements('.task-row').find(e => e.textContent.includes(meetingId));
      await wait('Trash disables retry and requeue', () => row() && button('重试失败阶段', row())?.disabled && button('重新入队', row())?.disabled);
      const blocked = await raw('task_get', { taskId });
      check(fixture.status + ': native capabilities deny Trash retry and requeue', !blocked.task.canRetry && !blocked.task.canRequeue && blocked.task.status === fixture.status);
      check(fixture.status + ': actual React queue disables Trash mutations', button('重试失败阶段', row()).disabled && button('重新入队', row()).disabled);
      await click('会议库'); await wait('Trash library controls', () => label('位置'));
      change(label('日期'), 'all'); change(label('位置'), 'trash'); change(label('搜索会议'), title);
      await wait('retained Trash meeting searchable', () => titles().length === 1 && titles()[0] === title);
      await click(title); await click('恢复会议'); await click('确认恢复', modal());
      await wait('restore committed', () => !modal() && !text().includes('此会议位于回收站'));
      await click('处理队列');
      await wait('restore returns exact eligible capabilities', () => row() && button('重试失败阶段', row())?.disabled === !fixture.retry && button('重新入队', row())?.disabled === !fixture.requeue);
      const restored = await raw('task_get', { taskId });
      check(fixture.status + ': restore preserves task and exact capabilities', restored.task.taskId === taskId && restored.task.canRetry === fixture.retry && restored.task.canRequeue === fixture.requeue);
      check(fixture.status + ': actual React queue reflects exact restored capabilities', button('重试失败阶段', row()).disabled === !fixture.retry && button('重新入队', row()).disabled === !fixture.requeue);
      evidence.push({ meetingId, taskId, status: fixture.status, eligibleRetry: fixture.retry, eligibleRequeue: fixture.requeue });
    }
    check('no playback requested', playCalls === 0);
    window.__ommFixtureResult = { ok: true, checks, playCalls, evidence, scope: 'F2 Trash UI and F5 title-only content; active-attempt/late-shutdown races remain Core regression evidence' };
  } catch (error) {
    window.__ommFixtureResult = { ok: false, checks, playCalls, error: String(error), meetingId, taskId, body: text().slice(0, 12000) };
  }
})();
