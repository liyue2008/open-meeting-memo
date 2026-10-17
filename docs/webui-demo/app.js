/* A closed, offline interaction prototype. Never talks to OMM or system services. */
(function () {
  'use strict';
  const D = window.DemoData;
  const STORAGE_KEY = 'omm-webui-fictional-v1';
  const clone = value => JSON.parse(JSON.stringify(value));
  const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const $ = selector => document.querySelector(selector);
  // Templates escape all data at interpolation sites. This second boundary permits only
  // inert UI markup, local anchors and numeric presentation styles before DOM insertion.
  function mount(element, markup) {
    const parsed = new DOMParser().parseFromString(markup, 'text/html');
    const tags = new Set('a article aside br button code dd details div dl dt form h1 h2 h3 i input label li mark nav ol option p section select small span strong summary table tbody td textarea th thead time tr ul'.split(' '));
    for (const node of [...parsed.body.querySelectorAll('*')]) {
      if (!tags.has(node.localName)) { node.remove(); continue; }
      for (const attr of [...node.attributes]) {
        const name = attr.name;
        if (name.startsWith('on') || ['src', 'srcset', 'action', 'formaction'].includes(name) ||
            (name === 'href' && !attr.value.startsWith('#')) ||
            (name === 'style' && !/^(?:height:\d+px|width:\d+%)$/.test(attr.value))) node.removeAttribute(name);
      }
    }
    element.replaceChildren(...parsed.body.childNodes);
  }

  const labels = { completed: '已完成', running: '处理中', queued: '排队中', failed: '需处理', empty: '无有效音频', recording: '录制中', cancelled: '已取消', transcribed: '逐字稿就绪' };
  const stageName = stage => ({ summary: '生成纪要', transcribe: '重新转写', all: '转写与纪要' }[stage] || '转写与纪要');
  const time = seconds => `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(Math.floor(seconds % 60)).padStart(2, '0')}`;
  const duration = seconds => seconds ? `${Math.floor(seconds / 60)} 分 ${seconds % 60 ? seconds % 60 + ' 秒' : ''}` : '—';
  const status = value => `<span class="status ${esc(value)}">${esc(labels[value] || value)}</span>`;
  const busy = m => ['running', 'queued', 'recording'].includes(m.status);
  let storageAvailable = true;
  function validState(value) {
    const defaults = D.fresh();
    const strings = items => Array.isArray(items) && items.every(item => typeof item === 'string');
    const artifact = item => item && Array.isArray(item.transcript) && item.transcript.every(t =>
      t && typeof t.text === 'string' && ['mic', 'system'].includes(t.source) && Number.isFinite(t.at) && t.at >= 0) &&
      (item.summary === null || (item.summary && typeof item.summary.intro === 'string' && strings(item.summary.decisions) &&
        Array.isArray(item.summary.topics) && item.summary.topics.every(t => t && typeof t.title === 'string' && typeof t.text === 'string' && Number.isFinite(t.at)) &&
        Array.isArray(item.summary.actions) && item.summary.actions.every(a => a && typeof a.text === 'string' && typeof a.owner === 'string' && typeof a.due === 'string')));
    return !!(value && value.schema === 2 && !!D.parseDay(value.anchorDate) && Array.isArray(value.meetings) && value.meetings.length <= 200 &&
      value.meetings.every(m => m && typeof m.id === 'string' && /^[a-z0-9-]+$/.test(m.id) &&
        ['title', 'date', 'time', 'source', 'note'].every(key => typeof m[key] === 'string') && !!D.parseDay(m.date) && strings(m.tags) && artifact(m) &&
        typeof m.audio === 'boolean' && Number.isFinite(m.duration) && m.duration >= 0 && Object.hasOwn(labels, m.status) &&
        Number.isInteger(m.segments) && m.segments >= 0 && Number.isInteger(m.completedSegments) && m.completedSegments >= 0 && m.completedSegments <= m.segments &&
        Array.isArray(m.versions) && m.versions.every(v => v && typeof v.name === 'string' && Object.hasOwn(labels, v.status) && ['all', 'summary', 'transcribe'].includes(v.stage) && (!v.snapshot || artifact(v.snapshot))) &&
        Number.isInteger(m.activeVersion) && m.activeVersion >= -1 && m.activeVersion < m.versions.length) &&
      new Set(value.meetings.map(m => m.id)).size === value.meetings.length &&
      value.config && Object.keys(defaults.config).every(key => typeof value.config[key] === typeof defaults.config[key]) &&
      Object.keys(value.config).length === Object.keys(defaults.config).length &&
      Array.isArray(value.events) && value.events.every(e => D.events.some(([key]) => key === e)) &&
      typeof value.auto === 'boolean' && typeof value.consent === 'boolean' && (!value.auto || value.consent) &&
      (value.recording === null || value.meetings.some(m => m.id === value.recording && m.status === 'recording')));
  }
  let state = D.fresh();
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const parsed = D.migrate(JSON.parse(stored));
      if (!validState(parsed)) throw new Error('invalid demo data');
      state = parsed;
      if (JSON.stringify(parsed) !== stored) localStorage.setItem(STORAGE_KEY, JSON.stringify(parsed));
    } else localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch { storageAvailable = false; }
  let route = { page: 'library', id: null };
  const defaultFilters = () => ({ query: '', scope: 'title', ...D.dateRange('seven'), preset: 'seven', source: '', status: '', sort: 'newest' });
  let filters = defaultFilters();
  let selected = new Set();
  let detailTab = 'summary';
  let transcriptQuery = '';
  let selectedVersion = null;
  let settingsGroup = '录音与检测';
  let draft = clone(state.config);
  let draftEvents = clone(state.events);
  let errors = {};
  let queueFilter = '';
  let player = { id: null, position: 0, speed: 1, playing: false };
  let modalReturnFocus = null;
  let modalSubmit = null;
  let pendingNavigation = null;
  let toastTimer;
  function save() {
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(state)); }
    catch { storageAvailable = false; }
    $('#storage-warning').hidden = storageAvailable;
    updateCounts();
  }
  function toast(text) {
    $('#toast').textContent = text;
    $('#toast').classList.add('visible');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => $('#toast').classList.remove('visible'), 3200);
  }
  const meeting = id => state.meetings.find(m => m.id === id);
  const dirty = () => JSON.stringify(draft) !== JSON.stringify(state.config) || JSON.stringify(draftEvents) !== JSON.stringify(state.events);
  function updateCounts() {
    $('#library-count').textContent = state.meetings.length;
    $('#queue-count').textContent = state.meetings.filter(m => ['queued', 'running', 'failed'].includes(m.status)).length || '';
  }
  function titleBlock(title, subtitle, actions = '', eyebrow = '') {
    return `<div class="page-heading"><div>${eyebrow ? `<div class="eyebrow">${eyebrow}</div>` : ''}<h1>${esc(title)}</h1><p>${esc(subtitle)}</p></div><div class="button-row">${actions}</div></div>`;
  }
  function empty(title, text, action = '') {
    return `<div class="empty-state"><div class="empty-icon" aria-hidden="true">▤</div><h2>${esc(title)}</h2><p>${esc(text)}</p>${action}</div>`;
  }
  function parseHash() {
    const parts = location.hash.slice(1).split('/');
    if (parts[0] === 'meeting') return { page: 'meeting', id: parts[1] || '' };
    return { page: ['library', 'queue', 'settings', 'diagnostics', 'help'].includes(parts[0]) ? parts[0] : 'library', id: null };
  }
  function go(hash) {
    if (location.hash === hash) return;
    location.hash = hash;
  }
  function applyRoute(next) {
    if (route.page === 'settings' && next.page !== 'settings' && dirty()) {
      pendingNavigation = next;
      history.replaceState(null, '', '#settings');
      showModal('还有未保存的设置', '<p>离开后会丢弃这次修改。设置只保存在本浏览器的演示数据中，不会更改 OMM。</p>', '放弃修改并离开', () => {
        draft = clone(state.config); draftEvents = clone(state.events); errors = {};
        const target = pendingNavigation; pendingNavigation = null;
        closeModal(); navigate(target);
      });
      return;
    }
    navigate(next);
  }
  function navigate(next) {
    if (next.id !== route.id || next.page !== 'meeting') { player.playing = false; selectedVersion = null; }
    if (next.page === 'meeting' && next.id !== route.id) { detailTab = 'summary'; transcriptQuery = ''; player = { id: next.id, position: 0, speed: 1, playing: false }; }
    route = next;
    history.replaceState(null, '', next.page === 'meeting' ? `#meeting/${next.id}` : `#${next.page}`);
    render();
    $('#main').focus({ preventScroll: true });
    window.scrollTo(0, 0);
  }
  window.addEventListener('hashchange', () => applyRoute(parseHash()));
  window.addEventListener('beforeunload', event => { if (dirty()) { event.preventDefault(); event.returnValue = ''; } });
  function render() {
    updateCounts();
    $('#storage-warning').hidden = storageAvailable;
    const names = { library: '会议库', meeting: '会议工作区', queue: '处理队列', settings: '设置', diagnostics: '状态与诊断', help: '使用帮助' };
    document.title = `${names[route.page]} · Open Meeting Memo 演示`;
    $('#breadcrumb').textContent = `工作空间 / ${names[route.page]}`;
    document.querySelectorAll('[data-nav]').forEach(el => {
      const active = el.dataset.nav === (route.page === 'meeting' ? 'library' : route.page);
      el.classList.toggle('active', active);
      if (active) el.setAttribute('aria-current', 'page'); else el.removeAttribute('aria-current');
    });
    mount($('#main'), ({ library: libraryView, meeting: detailView, queue: queueView, settings: settingsView, diagnostics: diagnosticsView, help: helpView }[route.page])());
  }
  function filteredMeetings() {
    if (!['all', 'custom'].includes(filters.preset)) Object.assign(filters, D.dateRange(filters.preset));
    return state.meetings.filter(m => {
      const haystack = filters.scope === 'content' ? [m.title, ...m.transcript.map(t => t.text), m.summary?.intro || '', ...(m.summary?.decisions || []),
        ...(m.summary?.topics || []).flatMap(t => [t.title, t.text]),
        ...(m.summary?.actions || []).flatMap(a => [a.text, a.owner, a.due])].join(' ') : m.title;
      return haystack.toLowerCase().includes(filters.query.trim().toLowerCase()) &&
        (!filters.from || m.date >= filters.from) && (!filters.to || m.date <= filters.to) &&
        (!filters.source || m.source === filters.source) && (!filters.status || m.status === filters.status);
    }).sort((a, b) => filters.sort === 'oldest' ? (a.date + a.time).localeCompare(b.date + b.time) : filters.sort === 'duration' ? b.duration - a.duration : (b.date + b.time).localeCompare(a.date + a.time));
  }
  function options(items, value, placeholder) {
    return `${placeholder !== undefined ? `<option value="">${esc(placeholder)}</option>` : ''}${items.map(item => {
      const [key, label] = Array.isArray(item) ? item : [item, item];
      return `<option value="${esc(key)}" ${String(key) === String(value) ? 'selected' : ''}>${esc(label)}</option>`;
    }).join('')}`;
  }
  function libraryView() {
    const list = filteredMeetings();
    const queueCount = state.meetings.filter(m => ['queued', 'running'].includes(m.status)).length;
    const failedCount = state.meetings.filter(m => m.status === 'failed').length;
    const active = state.recording && meeting(state.recording);
    return titleBlock('每一次讨论，都有迹可循。', '查找会议，回看决定，让下一步更清楚。', '<button data-action="import">导入音频样例</button><button class="primary" data-action="record">＋ 开始录制</button>', 'YOUR MEETINGS') +
      (active ? `<div class="record-banner"><p><strong>● 模拟录制中</strong>　${esc(active.title)}<br><small>没有访问麦克风或系统音频；停止后演示封存与排队。</small></p><button data-action="stop-record">停止并封存</button></div>` : '') +
      `<div class="overview-strip"><section class="welcome-card"><h2>留住讨论，把注意力还给会议。</h2><p>录音、逐字稿与纪要，收在同一个地方。<br>这里的故事与操作均为离线演示。</p></section><section class="queue-card"><h3><span class="dot ${failedCount ? 'orange' : ''}"></span>${queueCount} 场处理中 · ${failedCount} 场需要关注</h3><p>分段转写在本地进行，纪要使用已配置的 Pi。</p><button class="text-button" data-action="goto-queue">查看处理队列 →</button></section></div>
      <section class="panel" aria-label="会议库"><div class="filter-top"><div class="searchbox"><span aria-hidden="true">⌕</span><input id="meeting-query" aria-label="搜索会议" placeholder="搜索标题，找到那次讨论…" value="${esc(filters.query)}"></div><select id="search-scope" aria-label="搜索范围">${options([['title', '标题'], ['content', '标题与演示正文']], filters.scope)}</select><select id="filter-source" aria-label="会议来源">${options(['飞书', '手动录制', '导入样例'], filters.source, '所有来源')}</select><select id="filter-status" aria-label="处理状态">${options(Object.entries(labels), filters.status, '所有状态')}</select></div>
      <div class="date-shortcuts" role="group" aria-label="日期快捷筛选">${[['today', '今日'], ['week', '本周'], ['three', '最近3天'], ['seven', '最近7天']].map(([key, label]) => `<button data-action="date-preset" data-preset="${key}" aria-pressed="${filters.preset === key}">${label}</button>`).join('')}<span>${filters.preset === 'custom' ? '自定义日期' : filters.preset === 'all' ? '全部日期' : '包含今天 · 本地自然日'}</span></div>
      <div class="filter-bottom"><label for="filter-from">日期 <input type="date" id="filter-from" value="${esc(filters.from)}"></label><label for="filter-to">至 <input type="date" id="filter-to" value="${esc(filters.to)}"></label><button class="text-button" data-action="clear-filters">清除筛选</button><label class="sort" for="sort">排序 <select id="sort">${options([['newest', '最近会议优先'], ['oldest', '较早会议优先'], ['duration', '时长从长到短']], filters.sort)}</select></label></div>
      ${filters.from && filters.to && filters.from > filters.to ? '<div class="notice warn">开始日期晚于结束日期，请调整日期范围。</div>' : ''}
      <div id="library-results">${libraryResults(list)}</div></section>
      <p class="page-note">共 ${state.meetings.length} 场虚构会议。样例生成日：${esc(state.anchorDate)}；快捷日期按本机今日计算，样例过旧可恢复演示。正文检索、导入、编辑与删除为 WebUI 交互提案，能力差异见 <a href="#help">使用帮助</a>。</p>`;
  }
  function libraryResults(list) {
    const all = list.length > 0 && list.every(m => selected.has(m.id));
    const selection = selected.size ? `<div class="selection-bar"><span>已选 ${selected.size} 场 <button class="text-button" data-action="unselect">取消选择</button></span><div class="button-row"><button data-action="batch-export">导出演示文本</button><button data-action="batch-summary">重新生成纪要</button><button class="subtle-danger" data-action="batch-delete">删除</button></div></div>` : '';
    if (!list.length) return selection + empty(state.meetings.length ? '没有找到匹配的会议' : '还没有会议', state.meetings.length ? '换一个关键词，或清除筛选再看看。' : '开始一次模拟录制，或导入一份虚构音频样例。', `<button data-action="${state.meetings.length ? 'clear-filters' : 'import'}">${state.meetings.length ? '清除全部筛选' : '导入音频样例'}</button>`);
    return selection + `<div class="table-wrap"><table class="meeting-table"><thead><tr><th><input type="checkbox" id="select-all" aria-label="选择当前筛选结果的所有会议" ${all ? 'checked' : ''}></th><th>会议名称</th><th>时间 / 时长</th><th>来源</th><th>处理状态</th></tr></thead><tbody>${list.map(m => `<tr><td><input type="checkbox" data-select="${esc(m.id)}" aria-label="选择 ${esc(m.title)}" ${selected.has(m.id) ? 'checked' : ''}></td><td><button class="meeting-link" data-action="open-meeting" data-id="${esc(m.id)}">${esc(m.title)}</button><div class="meeting-sub">${m.tags.map(t => `<span class="tag">${esc(t)}</span>`).join('')}${m.summary ? '<span>纪要已整理</span>' : m.transcript.length ? `<span>${m.completedSegments < m.segments ? '部分' : '完整'}逐字稿可读</span>` : '<span>等待内容</span>'}</div></td><td>${esc(m.date.slice(5).replace('-', ' 月 '))} 日 ${esc(m.time)}<div class="meeting-sub">${duration(m.duration)}</div></td><td>${esc(m.source)}</td><td>${status(m.status)}</td></tr>`).join('')}</tbody></table></div><div class="table-footer"><span>显示 ${list.length} 场会议 · 全部结果已列出</span><span>资料仅保存在此浏览器演示中</span></div>`;
  }
  function refreshLibraryResults() {
    const list = filteredMeetings();
    $('#filter-from').value = filters.from;
    $('#filter-to').value = filters.to;
    mount($('#library-results'), libraryResults(list));
  }
  function currentArtifact(m) {
    if (selectedVersion !== null && m.versions[selectedVersion]?.snapshot) return m.versions[selectedVersion].snapshot;
    return m;
  }
  function detailView() {
    const m = meeting(route.id);
    if (!m) return empty('这场会议不存在', '可能已从演示中删除。恢复演示数据可以重新查看。', '<a href="#library">返回会议库</a>');
    const artifact = currentArtifact(m);
    const old = selectedVersion !== null;
    const completeText = m.completedSegments === m.segments && m.transcript.length > 0;
    const tabs = [['summary', '会议纪要'], ['transcript', '逐字稿'], ['info', '信息与版本']];
    return `<a class="back" href="#library">← 返回会议库</a><div class="page-heading detail-heading"><div><h1>${esc(m.title)}</h1><div class="meta-line"><span>${esc(m.date)} ${esc(m.time)}</span><span>${duration(m.duration)}</span><span>${esc(m.source)}</span>${status(m.status)}</div></div><div class="detail-actions"><button data-action="edit" data-id="${esc(m.id)}">编辑</button><button data-action="retranscribe" data-id="${esc(m.id)}" ${!m.audio || busy(m) ? 'disabled title="需要已封存音频且没有在途任务"' : ''}>重新转写</button><button data-action="resummary" data-id="${esc(m.id)}" ${!completeText || busy(m) ? 'disabled title="需要完整已提交逐字稿且没有在途任务"' : ''}>重新纪要</button><button class="subtle-danger" data-action="delete" data-id="${esc(m.id)}" ${busy(m) ? 'disabled title="先停止模拟录制或取消在途任务"' : ''}>删除</button></div></div>
      ${m.error ? `<div class="notice warn"><p>${esc(m.error)}<br>已有结果仍可阅读，重试只处理未完成阶段。</p><button data-action="retry" data-id="${esc(m.id)}">重试失败阶段</button></div>` : ''}
      ${['running', 'queued'].includes(m.status) ? `<div class="notice info"><p>${m.pendingStage === 'transcribe' ? '新一轮转写处理中，仍展示上次已提交的结果。' : m.completedSegments < m.segments ? `部分逐字稿 · ${m.completedSegments}/${m.segments} 段完成，尚未形成终稿。` : '转写已提交，正在等待生成纪要。'} 演示进度需手动推进；旧结果保留。</p><button data-action="goto-queue">查看队列 →</button></div>` : ''}
      ${m.status === 'recording' ? '<div class="notice info"><p>正在模拟录制，无硬件采集。停止后才演示尾段封存。</p><button data-action="stop-record">停止并封存</button></div>' : ''}
      ${old ? `<div class="notice"><p>正在阅读历史版本：${esc(m.versions[selectedVersion].name)}。只读，不会覆盖当前结果。</p><button data-action="latest-version">回到最新结果</button></div>` : ''}
      ${audioView(m)}
      <div class="tabs" role="tablist" aria-label="会议资料">${tabs.map(([key, text]) => `<button role="tab" id="tab-${key}" aria-controls="detail-panel" aria-selected="${detailTab === key}" tabindex="${detailTab === key ? 0 : -1}" class="${detailTab === key ? 'active' : ''}" data-tab="${key}">${text}${key === 'transcript' && artifact.transcript.length ? ` <small>${artifact.transcript.length}</small>` : ''}</button>`).join('')}</div>
      <div class="detail-grid"><section class="panel document" role="tabpanel" id="detail-panel" aria-labelledby="tab-${detailTab}">${detailTab === 'summary' ? summaryView(m, artifact, old) : detailTab === 'transcript' ? transcriptView(m, artifact) : infoView(m)}</section><aside class="detail-aside">${detailAside(m, artifact)}</aside></div>`;
  }
  function summaryView(m, artifact, old) {
    const summary = artifact.summary;
    if (!summary) return empty(m.status === 'empty' ? '这次没有可整理的内容' : '纪要还没准备好', m.status === 'failed' ? '逐字稿已保存。可以先阅读，或重试纪要阶段。' : m.status === 'empty' ? '没有有效音频，所以没有转写或纪要任务。' : '完整逐字稿提交后，由 Pi 生成一份会议纪要。', m.status === 'failed' ? `<button data-action="retry" data-id="${esc(m.id)}">重试纪要</button>` : '<button data-action="show-transcript">查看逐字稿</button>');
    return `<div class="doc-header"><h2>会议纪要</h2><div class="button-row"><button data-action="copy" data-artifact="summary">复制</button><button data-action="export" data-artifact="summary">导出 Markdown</button></div></div><p class="lead">${esc(summary.intro)}</p><section class="doc-section"><h3>关键决定</h3><ul class="decision-list">${summary.decisions.map(t => `<li>${esc(t)}</li>`).join('')}</ul></section><section class="doc-section"><h3>讨论要点</h3>${summary.topics.map(t => `<article class="topic"><h3>${esc(t.title)}</h3><p>${esc(t.text)}</p><button class="timestamp" data-action="locate" data-at="${t.at}">${time(t.at)} ↗ 查看原文</button></article>`).join('')}</section><p class="version-note">虚构演示内容 · ${old ? '历史结果，只读' : '当前已提交结果'} · 纪要请结合原文核对</p>`;
  }
  function detailAside(m, artifact) {
    return `<section class="panel side-card"><h3>行动项 <span class="tag">${artifact.summary?.actions.length || 0}</span></h3>${artifact.summary ? `<ul class="actions-list">${artifact.summary.actions.map(a => `<li class="action-item"><div><span class="action-text">${esc(a.text)}</span><small>${esc(a.owner)} · ${esc(a.due)}</small></div></li>`).join('')}</ul><p class="version-note">纪要原文信息，仅供阅读；OMM 不管理行动项跟进。</p>` : '<p>纪要生成后，在这里阅读行动项。</p>'}</section>
      <section class="panel side-card"><h3>处理与来源 <a href="#queue">查看队列</a></h3><ol class="mini-steps"><li class="${m.audio ? '' : 'pending'}">${m.audio ? '公开混音已封存（样例）' : '暂无有效音频'}</li><li class="${m.completedSegments === m.segments && m.transcript.length ? '' : 'pending'}">本地转写 ${m.completedSegments} / ${m.segments} 段</li><li class="${m.summary ? '' : 'pending'}">${m.summary ? '纪要已提交' : '纪要等待完成'}</li></ol><dl class="definition"><div><dt>语音识别</dt><dd>FireRedASR INT8<br><small>模型状态未检测</small></dd></div><div><dt>纪要模型</dt><dd>demo-text-model<br><small>虚构配置</small></dd></div><div><dt>录音格式</dt><dd>48 kHz · Opus<br><small>48 kbps / 演示</small></dd></div></dl></section>`;
  }
  function highlighted(text) {
    if (!transcriptQuery.trim()) return esc(text);
    const q = transcriptQuery.trim();
    const lower = text.toLowerCase();
    let cursor = 0; let result = ''; let found;
    while ((found = lower.indexOf(q.toLowerCase(), cursor)) !== -1) {
      result += esc(text.slice(cursor, found)) + '<mark>' + esc(text.slice(found, found + q.length)) + '</mark>';
      cursor = found + q.length;
    }
    return result + esc(text.slice(cursor));
  }
  function transcriptLines(artifact) {
    const lines = artifact.transcript.filter(t => t.text.toLowerCase().includes(transcriptQuery.trim().toLowerCase()));
    return lines.length ? lines.map(t => `<article class="transcript-line" data-line-at="${t.at}"><button class="timestamp" data-action="seek" data-at="${t.at}" aria-label="定位到 ${time(t.at)}">${time(t.at)}</button><div><small>${t.source === 'mic' ? '麦克风声源' : '系统声源'} · 非说话人识别</small><p>${highlighted(t.text)}</p></div></article>`).join('') : empty('没有匹配的片段', '换个关键词，或清空搜索。', '<button data-action="clear-transcript">清空搜索</button>');
  }
  function transcriptView(m, artifact) {
    if (!artifact.transcript.length) return empty('暂时没有逐字稿', m.status === 'empty' ? '没有有效音频，不生成空白终稿。' : '尚无已提交片段；这不表示音频没有内容。', '<a href="#queue">查看处理进度 →</a>');
    return `<div class="doc-header"><h2>${m.completedSegments < m.segments && selectedVersion === null ? '部分逐字稿' : '逐字稿'}</h2><div class="button-row"><button data-action="copy" data-artifact="transcript">复制</button><button data-action="export" data-artifact="transcript">导出文本</button></div></div><div class="transcript-tools"><div class="searchbox"><span aria-hidden="true">⌕</span><input id="transcript-query" aria-label="搜索逐字稿" placeholder="搜索这场会议的逐字稿" value="${esc(transcriptQuery)}"></div><small id="transcript-count"></small></div><div id="transcript-lines">${transcriptLines(artifact)}</div>`;
  }
  function infoView(m) {
    return `<div class="doc-header"><h2>会议信息与版本</h2><span class="proposal">版本阅读交互提案</span></div><dl class="definition"><div><dt>会议标题</dt><dd>${esc(m.title)}</dd></div><div><dt>来源 / 时间</dt><dd>${esc(m.source)} · ${esc(m.date)} ${esc(m.time)}</dd></div><div><dt>备注</dt><dd>${esc(m.note) || '未填写'}</dd></div><div><dt>标签</dt><dd>${m.tags.map(esc).join('、') || '无'}</dd></div><div><dt>采集缺口</dt><dd>${m.status === 'empty' ? '未采集' : '样例未设置缺口，非真实检测'}</dd></div></dl><section class="doc-section"><h3>处理记录</h3><p class="muted">重新处理创建新的尝试。旧音频和已提交结果保留；新结果成功后才作为当前阅读版本。</p>${m.versions.length ? m.versions.map((v, i) => `<div class="version-item"><div><strong>${esc(v.name)}</strong> ${status(v.status)}<small>${stageName(v.stage)} · ${i === m.activeVersion ? '当前已提交结果' : v.snapshot ? '保留的历史结果' : '尚无提交结果'}${v.model ? ` · ${esc(v.model)}` : ''}</small></div>${v.snapshot ? `<button data-action="read-version" data-version="${i}">阅读此版</button>` : ''}</div>`).reverse().join('') : '<p class="muted">没有任务：没有有效音频帧。</p>'}</section>`;
  }
  function audioView(m) {
    if (!m.audio) return '<div class="audio-panel"><p class="muted">没有可播放的录音。未访问任何音频文件。</p></div>';
    return `<section class="audio-panel" aria-label="模拟录音播放器"><div class="audio-top"><strong>录音回听 <span class="tag">模拟播放，无声</span></strong><span>公开混音 · 不读取音频文件</span></div><div class="waveform" aria-hidden="true">${Array.from({ length: 160 }, (_, i) => `<i style="height:${6 + ((i * 17 + i * i) % 23)}px"></i>`).join('')}</div><div class="audio-controls"><button class="jump" data-action="back-15" aria-label="后退15秒">↶ 15</button><button class="play" data-action="play" aria-label="${player.playing ? '暂停模拟播放' : '开始模拟播放'}">${player.playing ? 'Ⅱ' : '▶'}</button><button class="jump" data-action="forward-15" aria-label="前进15秒">15 ↷</button><input id="audio-progress" type="range" min="0" max="${m.duration}" value="${Math.min(player.position, m.duration)}" aria-label="模拟播放进度（秒）"><time id="audio-time">${time(player.position)} / ${time(m.duration)}</time><select id="audio-speed" aria-label="模拟播放倍速">${options([['0.75', '0.75×'], ['1', '1×'], ['1.25', '1.25×'], ['1.5', '1.5×'], ['2', '2×']], player.speed)}</select></div></section>`;
  }
  function queueView() {
    const tasks = state.meetings.filter(m => m.versions.length && (!queueFilter || m.status === queueFilter));
    return titleBlock('处理队列', '一场会议一条记录。查看分段进度、处理结果和恢复入口。', '<button data-action="advance-all">推进所有演示任务 →</button>', 'PROCESSING') +
      '<div class="notice info"><p>这是可控的进度演示，不会后台运行模型。点击「推进一步」体验排队 → 转写 → 纪要 → 完成；也可注入失败、重试或取消。</p></div>' +
      `<div class="queue-controls"><label for="queue-filter">处理状态</label><select id="queue-filter">${options(Object.entries(labels).filter(([key]) => !['empty', 'recording', 'transcribed'].includes(key)), queueFilter, '所有处理记录')}</select><span class="muted">${tasks.length} 场会议</span></div><section class="panel">${tasks.length ? tasks.map(m => {
        const v = m.versions[m.versions.length - 1];
        const processed = m.pendingSegments ?? m.completedSegments;
        const pct = m.status === 'completed' ? 100 : m.status === 'queued' ? 0 : (m.pendingStep ?? 0) < 2 && processed < m.segments ? Math.round(processed / Math.max(1, m.segments) * 65) : 80;
        return `<article class="queue-item"><div class="queue-item-head"><div><button class="meeting-link" data-action="open-meeting" data-id="${esc(m.id)}">${esc(m.title)} ↗</button><p>${esc(m.date)} · ${esc(v.name)} · ${stageName(v.stage)}</p></div>${status(m.status)}</div><div class="progress" aria-label="${esc(m.title)} 演示进度 ${pct}%"><span style="width:${pct}%"></span></div><p>${m.status === 'completed' ? '逐字稿和纪要均已提交。' : m.status === 'cancelled' ? '演示任务已取消，已有音频和已提交结果保留。' : `本次转写 ${processed}/${m.segments} 段 · ${m.pendingStage === 'transcribe' ? '重新处理中，仍可阅读上次提交的结果' : m.completedSegments < m.segments ? '仅已完成段可读；未完成内容不是空白终稿' : '最终转写已提交，纪要待完成'}`}</p>${m.error ? `<p class="error-text">${esc(m.error)}</p>` : ''}<div class="button-row">${['queued', 'running'].includes(m.status) ? `<button class="primary" data-action="advance" data-id="${esc(m.id)}">推进一步</button><button data-action="inject-failure" data-id="${esc(m.id)}">模拟失败</button><button data-action="cancel-task" data-id="${esc(m.id)}">取消任务</button>` : ['failed', 'cancelled'].includes(m.status) ? `<button data-action="retry" data-id="${esc(m.id)}">${m.status === 'failed' ? '重试未完成阶段' : '重新排队'}</button>` : `<button data-action="open-meeting" data-id="${esc(m.id)}">阅读会议资料</button>`}<button class="text-button" data-action="task-details" data-id="${esc(m.id)}">处理详情</button></div></article>`;
      }).join('') : empty('这里暂时没有处理记录', '调整状态筛选，或从会议工作区发起处理。', '<button data-action="clear-queue">查看全部记录</button>')}</section><p class="page-note">取消与可控推进是演示交互；现有 CLI 的等待超时不会取消持久任务。恢复策略需在真实开发前确认。</p>`;
  }
  function fieldError(field, value) {
    if (field.type === 'checkbox') return '';
    if (field.type === 'number') {
      const n = Number(value);
      if (value === '' || !Number.isSafeInteger(n)) return '请输入整数。';
      if (n < field.min || (field.max !== undefined && n > field.max)) return `请输入 ${field.min}${field.max !== undefined ? '–' + field.max : ' 或更大'}。`;
    } else if (field.options) {
      if (!field.options.includes(Number(value))) return '请选择支持的码率。';
    } else if (field.pathList) {
      if (String(value).split('\n').filter(Boolean).some(path => !path.startsWith('/'))) return '每行都需要绝对路径（以 / 开头）。';
    } else if (field.path) {
      if (!value && field.optional) return '';
      if (!String(value).startsWith('/')) return '请输入绝对路径（以 / 开头）；这里不检查文件是否存在。';
    } else if (!String(value).trim()) return '此项不能为空。';
    return '';
  }
  function validateDraft() {
    errors = {};
    for (const f of D.fields) { const error = fieldError(f, draft[f.key]); if (error) errors[f.key] = error; }
    return !Object.keys(errors).length;
  }
  function settingField(f) {
    const id = `setting-${f.key.replaceAll('.', '-')}`;
    const value = draft[f.key];
    const error = errors[f.key];
    const attrs = `id="${id}" data-setting="${f.key}" aria-describedby="${id}-hint ${id}-error" ${error ? 'aria-invalid="true"' : ''}`;
    let input = f.options ? `<select ${attrs}>${options(f.options, value)}</select>` : f.type === 'textarea' ? `<textarea ${attrs} rows="2">${esc(value)}</textarea>` : f.type === 'checkbox' ? `<input ${attrs} type="checkbox" ${value ? 'checked' : ''}>` : `<input ${attrs} type="${f.type || 'text'}" value="${esc(value)}" ${f.type === 'number' ? `step="1" min="${f.min}" ${f.max !== undefined ? `max="${f.max}"` : ''}` : ''} ${!f.optional ? 'required' : ''} autocomplete="off">`;
    return `<div class="field ${f.path || f.pathList ? 'path-field' : ''}"><label for="${id}">${esc(f.label)}</label><div class="input-wrap">${input}${f.unit ? `<span>${f.unit}</span>` : ''}</div><code>${esc(f.key)}</code><small id="${id}-hint">${esc(f.hint || (f.path ? '虚构绝对路径；不读取、执行或验证本机文件。' : ''))}</small><span class="error-text" id="${id}-error">${esc(error || '')}</span></div>`;
  }
  function settingsView() {
    const groups = ['录音与检测', '本地转写', 'Pi 纪要', '存储', '通知'];
    return titleBlock('让 OMM 适合你的工作方式', '设置仅修改虚构演示状态，不读取或写入真实 config.json。', '', 'PREFERENCES') +
      `<div class="settings-layout"><nav class="settings-nav" aria-label="设置分组">${groups.map(g => `<button data-settings-group="${g}" class="${settingsGroup === g ? 'active' : ''}" ${settingsGroup === g ? 'aria-current="true"' : ''}>${g}${D.fields.some(f => f.group === g && errors[f.key]) ? ' · 有错误' : ''}</button>`).join('')}</nav><div><form id="settings-form" novalidate><section class="panel settings-panel"><h2>${settingsGroup}</h2><p>${{ '录音与检测': '录音参数和飞书检测。修改不会触发录音或授予权限。', '本地转写': '音频仅在本地转写，不静默更换引擎或模型。', 'Pi 纪要': '仅向配置的 Pi 模型传入逐字稿与必要时间信息，不传音频。', '存储': '按 XDG 目录分类。这里展示默认示例，不是本机检测结果。', '通知': '仅通知生命周期和安全原因，不包含逐字稿或纪要正文。' }[settingsGroup]}</p>
      ${settingsGroup === '录音与检测' ? `<div class="switch-row"><div><h3>自动录音 ${state.auto ? '<span class="tag">演示已开启</span>' : ''}</h3><p>默认关闭。需要知情同意与飞书强信号；手动停止会抑制当前会议自动重启。</p></div><button data-action="toggle-auto" type="button">${state.auto ? '关闭演示策略' : '开启演示策略'}</button></div><p class="page-note">自动录音独立确认并立即保存到演示状态，不随下方参数保存。系统权限、飞书认证：未检测。</p><br>` : ''}
      ${settingsGroup === '本地转写' ? '<div class="notice info"><p>固定模型：sherpa-onnx 1.13.2 / FireRedASR INT8 + Silero VAD。未检测安装、哈希或推理能力。<a href="#diagnostics">查看模型与依赖说明</a></p></div>' : ''}
      ${settingsGroup === 'Pi 纪要' ? '<div class="notice warn"><p>认证状态：未检测。密钥、令牌与密码不在此填写；请使用 Pi 官方支持的方式配置 OMM 专用目录。云端模型可能收到转写文本；逐字稿提交后直接生成纪要，不增加外发审批开关。</p></div>' : ''}
      ${settingsGroup === '存储' ? `<div class="path-list">${[['配置', '~/.config/omm/'], ['会议、音频、文本和模型', '~/.local/share/omm/'], ['任务与恢复状态', '~/.local/state/omm/'], ['可重建临时文件', '~/.cache/omm/'], ['运行目录', '$XDG_RUNTIME_DIR/omm 或 macOS 用户临时区域']].map(([label, path]) => `<div><strong>${label}</strong><code>${esc(path)}</code></div>`).join('')}</div><div class="notice"><p>目录只读展示。迁移持久根需显式重配服务，不在表单里移动文件。私有目录 0700、敏感文件 0600；不会一键清理会议或未完成检查点。</p></div><button type="button" data-action="storage-info">了解存储与保留规则</button>` : `<div class="form-grid">${D.fields.filter(f => f.group === settingsGroup).map(settingField).join('')}</div>`}
      ${settingsGroup === '通知' ? `<section class="doc-section"><h3>通知事件</h3><div class="check-grid">${D.events.map(([key, label]) => `<label><input type="checkbox" data-notification-event="${key}" ${draftEvents.includes(key) ? 'checked' : ''}>${label}</label>`).join('')}</div><div class="notice info"><p>macOS 使用系统 AppleScript 宿主，没有独立 OMM 通知身份。授权和横幅可见性：未知；本演示不发送任何桌面通知。</p></div><button type="button" data-action="notification-preview">预览通知卡片</button></section>` : ''}
      </section><div id="settings-save" class="settings-save ${dirty() ? 'dirty' : ''}"><span id="save-state" role="status">${dirty() ? '● 有未保存的修改' : '设置已保存 · 仅演示'}</span><div class="button-row"><button type="button" data-action="discard-settings" ${!dirty() ? 'disabled' : ''}>放弃修改</button><button type="submit" class="primary" ${!dirty() ? 'disabled' : ''}>保存设置</button></div></div><p class="page-note">生效时机：新处理尝试使用新配置快照；已有或在途任务保持原快照。码率只影响随后入队的新段。录音/检测策略由服务协调应用，具体网页保存接口待确认。</p></form></div></div>`;
  }
  function updateDirtyUi() {
    const changed = dirty();
    $('#settings-save')?.classList.toggle('dirty', changed);
    if ($('#save-state')) $('#save-state').textContent = changed ? '● 有未保存的修改' : '设置已保存 · 仅演示';
    const discard = $('[data-action="discard-settings"]'); if (discard) discard.disabled = !changed;
    const submit = $('#settings-form [type=submit]'); if (submit) submit.disabled = !changed;
  }
  function diagnosticsView() {
    return titleBlock('知道当前状态，再决定下一步', '静态页面没有连接本机服务。所有真实状态均未检测。', '<button data-action="diagnostic-report">导出演示诊断</button>', 'STATUS & DIAGNOSTICS') +
      '<div class="notice warn"><p>未连接真实 OMM。演示中的“完成”仅指虚构会议处理结果，不能说明本机已获得权限、模型已安装或 Pi 已认证。</p></div>' +
      `<div class="diagnostic-grid">${[
        ['本地服务', '未连接', '用户级后台服务负责录音与任务。注册成功不等于就绪；停止服务需先封存录音。', 'service-info', '查看生命周期说明'],
        ['录音与系统权限', '未检测', '麦克风、系统音频和必要的窗口权限需用户在 macOS 原生设置中授权；未知不是已拒绝。', 'permission-info', '查看授权指引'],
        ['本地语音模型与编码', '未检测', '固定原生库、模型与编码工具需显式安装和校验。绝不在打开页面时联网下载。', 'model-info', '查看依赖清单'],
        ['Pi 与认证', '未检测', '使用 OMM 专用 Pi 配置。仅传文本；模型端点决定是否访问网络，失败不切换提供方。', 'pi-info', '查看认证说明'],
        ['存储与路径', '示例值', '保留音频、已提交产物和恢复检查点。改变 XDG 根需显式重配，不静默迁移。', 'storage-info', '查看保留规则'],
        ['桌面通知', '可见性未知', '通过系统脚本宿主尽力递送，调用成功不等于用户看见。这里不发通知。', 'notification-preview', '查看页面内预览']
      ].map(([title, label, text, action, button]) => `<section class="panel diagnostic-card"><h3>${title}</h3><span class="status">${label}</span><p>${text}</p><button data-action="${action}">${button}</button></section>`).join('')}</div><section class="panel wide-panel section-spaced"><h3>通知历史 · 虚构示例</h3><dl class="definition"><div><dt>10-16 14:00 · 纸舟阅读计划</dt><dd>录音开始 · 调用成功 / 可见性未知</dd></div><div><dt>10-15 15:30 · 小岛书店</dt><dd>纪要失败 · 演示重试待处理</dd></div></dl><p class="page-note">没有发送桌面通知。诊断导出只含固定状态说明和演示计数，不含正文、路径或凭据。</p></section>`;
  }
  function helpView() {
    return titleBlock('从一次讨论，到清晰的下一步', '先确认功能与交互，再讨论真实开发。', '<button data-action="reset">恢复演示数据</button>', 'A LITTLE GUIDE') +
      '<section class="help-hero"><h2>这是一份可以点的设计稿。</h2><p>会议和人物全部虚构。没有后端、真实录音、音频播放、权限请求、通知发送或模型调用。播放器只推动无声时间轴。关闭后是否保留演示取决于浏览器的本地存储支持。</p></section>' +
      `<div class="help-columns"><section class="panel help-block"><h3>试试这些完整路径</h3><ul><li>在会议库搜索「纸舟」，按日期、来源和状态筛选；多选并导出。</li><li>打开体验评审，阅读决定和行动项；从时间标记跳到逐字稿，拖动无声播放器。</li><li>打开「小岛书店」，查看错误并重试纪要；在队列里推进、注入失败或取消。</li><li>重新转写或重新纪要，保留旧结果；完成后在「信息与版本」阅读历史版本。</li><li>编辑会议、模拟录制并封存，或导入样例。删除需确认，在途任务需先取消。</li><li>修改设置，体验校验、放弃、保存和离开提醒；帮助页可恢复初始数据。</li></ul></section><section class="panel help-block"><h3>常见问题</h3><details open><summary>本地保存，是否就完全离线？</summary><p>本演示完全离线。真实 OMM 的录音与转写在本地；Pi 若使用云模型，转写文本会发送到已配置的端点，不发送音频。</p></details><details><summary>为什么逐字稿只有一部分？</summary><p>封存片段逐段提交。部分结果明确标记，不是终稿；零个完成片段也不代表录音没有内容。</p></details><details><summary>重新处理会覆盖旧资料吗？</summary><p>已有 CLI force 会创建新尝试并保留旧产物。本演示用版本阅读器呈现：新结果成功后切换为当前，失败或取消保留原结果。</p></details><details><summary>如何配置认证或申请权限？</summary><p>此处不收集密钥。真实认证需在专用 Pi 中完成；系统权限由用户通过原生设置批准，网页接入方式待确认。</p></details><details><summary>哪些东西不在此次范围？</summary><p>云协作、多用户、远程访问、分享链接、实时字幕、说话人识别、日历同步、真实安装或服务控制均不开发。</p></details></section></div>
      <section class="panel wide-panel section-spaced"><h2>从命令行能力到网页体验</h2><table class="capability-table"><thead><tr><th>体验</th><th>已有 CLI 能力</th><th>本演示新增 / 待确认</th></tr></thead><tbody><tr><td>会议库与阅读</td><td><code>meeting list / show / read</code><br>标题、起始日期、来源 ID、状态；部分/完整文本</td><td>正文检索、结束日期、排序、多选、复制/导出、无声播放器</td></tr><tr><td>会议管理</td><td><code>record start / stop / auto</code></td><td>元数据编辑、删除、导入样例；删除范围/保留策略待确认</td></tr><tr><td>处理与版本</td><td><code>process --stage / --force / --retry</code><br><code>task list / show / wait</code></td><td>可视队列、历史阅读、批量纪要与任务取消；取消接口尚无 CLI</td></tr><tr><td>配置</td><td><code>config show / set / validate</code><br><code>paths</code></td><td>分组表单、草稿校验和保存流程；网页保存原子性与并发冲突待确认</td></tr><tr><td>状态与诊断</td><td><code>doctor / service / permissions / model / codec / notifications</code></td><td>只做未连接说明和页面内通知预览，不执行安装、下载或权限操作</td></tr></tbody></table></section><p class="page-note">需用户确认：页面布局与使用路径；导入格式与来源；删除是否需要回收站；重处理版本选择；批量操作和取消范围；本地网页访问安全与配置生效细节。上述均不是已批准的后端开发承诺。</p>`;
  }
  function showModal(title, body, confirmText, onConfirm, danger = false) {
    const modal = $('#modal');
    if (modal.open) modal.close();
    modalReturnFocus = document.activeElement;
    modalSubmit = onConfirm;
    mount(modal, `<form id="modal-form"><div class="modal-header"><h2 id="modal-title">${esc(title)}</h2><button class="modal-close" type="button" data-action="close-modal" aria-label="关闭对话框">×</button></div><div class="modal-body">${body}<p id="modal-error" class="error-text" role="alert"></p></div><div class="modal-footer"><button type="button" data-action="close-modal">${confirmText ? '取消' : '关闭'}</button>${confirmText ? `<button type="submit" class="${danger ? 'danger' : 'primary'}">${esc(confirmText)}</button>` : ''}</div></form>`);
    modal.showModal();
    const focus = modal.querySelector('[autofocus]') || (danger ? modal.querySelector('[data-action="close-modal"]') : modal.querySelector('[type=submit]')) || modal.querySelector('button');
    focus?.focus();
  }
  function closeModal() {
    $('#modal').close(); modalSubmit = null; pendingNavigation = null;
    if (modalReturnFocus?.isConnected) modalReturnFocus.focus();
  }
  $('#modal').addEventListener('cancel', event => { event.preventDefault(); closeModal(); });
  function confirmDelete(ids) {
    const items = ids.map(meeting).filter(Boolean);
    if (!items.length) return;
    const active = items.filter(busy);
    if (active.length) {
      showModal('先处理在途工作', `<p>选中的 ${active.length} 场会议仍在模拟录制或处理中。请先停止录制或在队列取消，再删除。没有删除任何会议。</p>`, '前往处理队列', () => { closeModal(); go('#queue'); });
      return;
    }
    showModal(`删除 ${items.length === 1 ? '这场' : items.length + ' 场'}会议？`, `<p>${items.map(m => esc(m.title)).join('<br>')}</p><div class="notice warn"><p>从本浏览器的演示库移除会议、样例音频标记和文本版本，无法单条撤销；可以恢复整套演示数据。不会访问或删除真实文件。真实删除/回收站策略待确认。</p></div>`, '确认删除', () => {
      state.meetings = state.meetings.filter(m => !ids.includes(m.id)); ids.forEach(id => selected.delete(id)); save(); closeModal();
      if (route.page === 'meeting') go('#library'); else render();
      toast('会议已从演示库删除');
    }, true);
  }
  function editMeeting(m) {
    showModal('编辑会议', `<p>修改演示中的标题、备注与标签，不改变音频、逐字稿或处理结果。</p><div class="field"><label for="edit-title">会议标题 *</label><input id="edit-title" maxlength="80" required value="${esc(m.title)}" autofocus></div><div class="field"><label for="edit-note">备注</label><textarea id="edit-note" rows="3" maxlength="400">${esc(m.note)}</textarea></div><div class="field"><label for="edit-tags">标签</label><input id="edit-tags" value="${esc(m.tags.join('，'))}" maxlength="80"><small>用逗号分隔，最多 5 个。</small></div>`, '保存修改', () => {
      const title = $('#edit-title').value.trim();
      if (!title) { $('#modal-error').textContent = '请填写会议标题。'; $('#edit-title').focus(); return; }
      m.title = title; m.note = $('#edit-note').value.trim(); m.tags = $('#edit-tags').value.split(/[,，]/).map(s => s.trim()).filter(Boolean).slice(0, 5);
      save(); closeModal(); render(); toast('会议信息已保存 · 仅演示');
    });
  }
  function preserveCurrent(m) {
    if (m.activeVersion >= 0 && m.versions[m.activeVersion]) m.versions[m.activeVersion].snapshot = clone({ transcript: m.transcript, summary: m.summary });
  }
  function enqueue(m, stage) {
    preserveCurrent(m);
    m.versions.push({ name: `第 ${m.versions.length + 1} 次处理`, stage, status: 'queued', model: state.config['pi.model'] });
    m.status = 'queued'; m.error = null;
    m.pendingStage = stage; m.pendingStep = stage === 'summary' ? 2 : 0;
    // Keep committed artifacts readable. Draft progress is separate until successful completion.
    m.pendingSegments = stage === 'summary' ? m.segments : 0;
  }
  function requestProcess(ids, stage) {
    const items = ids.map(meeting).filter(Boolean);
    const eligible = items.filter(m => !busy(m) && (stage === 'summary' ? m.transcript.length && m.completedSegments === m.segments : m.audio));
    if (!eligible.length) { showModal('暂时无法发起处理', '<p>重新转写需要已封存音频；重新纪要需要完整已提交逐字稿。正在录制或有在途任务的会议不能重复发起。</p>', null); return; }
    showModal(stage === 'summary' ? '重新生成纪要' : '重新转写录音', `<p>${eligible.map(m => esc(m.title)).join('<br>')}</p><div class="notice"><p>${stage === 'summary' ? '输入：复用完整已提交逐字稿，不重新识别音频。使用当前 Pi 配置（虚构模型）生成新纪要。' : '输入：已封存的本地音频，使用当前本地转写配置重新识别；完成后自动衔接 Pi 纪要。不重新编码原录音。'}<br>影响：创建新尝试，旧产物保留。新结果成功后才切换当前阅读结果；失败或取消仍可读旧版。</p></div>${eligible.length < items.length ? `<p>选中 ${items.length} 场，${items.length - eligible.length} 场因在途任务或输入不足跳过。</p>` : ''}<p>演示只排队，不调用模型；请到处理队列手动推进。</p>`, `确认排队 ${eligible.length} 场`, () => {
      eligible.forEach(m => enqueue(m, stage)); save(); closeModal(); selectedVersion = null; render(); toast('已加入演示队列，旧结果保留');
    });
  }
  function sampleTranscript(m) {
    return clone(D.sampleTranscript).map(t => ({ ...t, at: Math.floor(t.at / 1860 * m.duration) }));
  }
  function sampleSummary(m) {
    const result = clone(D.sampleSummary);
    result.topics.forEach(t => { t.at = Math.floor(t.at / 1860 * m.duration); });
    return result;
  }
  function advance(m) {
    if (!['queued', 'running'].includes(m.status)) return;
    const v = m.versions[m.versions.length - 1];
    if (m.status === 'queued') { m.status = 'running'; v.status = 'running'; return; }
    const step = m.pendingStep ?? (m.completedSegments < m.segments ? 0 : 2);
    if (step < 2) {
      m.pendingStep = step + 1;
      m.pendingSegments = step === 0 ? Math.floor(m.segments / 2) : m.segments;
      // A committed final remains readable even when summary generation has not succeeded.
      if (!m.transcript.length || m.completedSegments < m.segments) {
        m.completedSegments = m.pendingSegments;
        m.transcript = m.completedSegments ? sampleTranscript(m).slice(0, step === 0 ? 4 : 10) : [];
      }
      return;
    }
    preserveCurrent(m);
    if ((m.pendingStage || v.stage) !== 'summary' || !m.transcript.length) m.transcript = sampleTranscript(m);
    m.summary = sampleSummary(m);
    if (m.id !== 'paper') m.summary.intro = `「${m.title}」的虚构演示结果。以下采用统一的纸舟样例内容展示文档结构，并非对真实录音的识别或总结。`;
    m.completedSegments = m.segments; m.status = 'completed'; m.error = null;
    v.status = 'completed'; v.snapshot = clone({ transcript: m.transcript, summary: m.summary }); m.activeVersion = m.versions.length - 1;
    delete m.pendingStep; delete m.pendingSegments; delete m.pendingStage;
  }
  function retry(m) {
    if (!['failed', 'cancelled'].includes(m.status)) return;
    showModal('重试未完成的阶段', `<p>${esc(m.title)}</p><p>保留有效检查点、录音和已提交结果，重新排队失败或取消的阶段。真实任务使用原配置快照；如果需要更新 Pi 配置，应从会议工作区「重新纪要」创建新尝试。</p>`, '重试演示任务', () => {
      m.status = 'queued'; m.error = null;
      m.pendingStep = m.pendingStep ?? (m.completedSegments === m.segments ? 2 : 0);
      m.versions[m.versions.length - 1].status = 'queued'; save(); closeModal(); render(); toast('未完成阶段已重新排队');
    });
  }
  function createMeeting(title, source, recording) {
    const now = new Date();
    const id = `sample-${Date.now()}-${Math.floor(Math.random() * 10000)}`;
    const m = { id, title, source, date: D.localDay(now), time: `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`, duration: recording ? 0 : 960, status: recording ? 'recording' : 'queued', note: '新建的虚构演示会议，未读取任何文件或采集音频。', tags: ['演示样例'], transcript: [], summary: null, audio: !recording, segments: recording ? 0 : 1, completedSegments: 0, versions: recording ? [] : [{ name: '初次处理', status: 'queued', stage: 'all' }], activeVersion: -1 };
    state.meetings.unshift(m); if (recording) state.recording = id;
    return m;
  }
  function recordingModal() {
    if (state.recording) { go(`#meeting/${state.recording}`); toast('已有一场模拟录制，请先停止'); return; }
    showModal('开始一次模拟录制', '<p>可体验手动录制 → 停止封存 → 本地转写 → 纪要的使用路径。不会请求权限、打开麦克风或捕捉系统声音。</p><div class="field"><label for="new-title">会议标题 *</label><input id="new-title" value="我的演示会议" maxlength="80" required autofocus></div><label class="inline-label"><input id="record-consent" type="checkbox" required>我知道这里仅模拟录制，不采集任何音频。</label><p class="page-note">真实录制会包含麦克风与系统其他应用声音，需用户知情并完成原生权限授权。</p>', '开始模拟录制', () => {
      const title = $('#new-title').value.trim();
      if (!title || !$('#record-consent').checked) { $('#modal-error').textContent = '请填写标题并确认模拟边界。'; return; }
      const m = createMeeting(title, '手动录制', true); save(); closeModal(); go(`#meeting/${m.id}`); toast('模拟录制已开始 · 没有硬件采集');
    });
  }
  function importModal() {
    showModal('导入音频样例', '<p>导入是 WebUI 新能力提案。这里不打开文件选择器、不读取你的文件，只创建一条虚构样例。</p><div class="field"><label for="import-title">会议标题 *</label><input id="import-title" value="咖啡地图 · 样例讨论" maxlength="80" required autofocus></div><label class="radio-option"><input type="radio" name="import-sample" value="normal" checked>16 分钟混音样例<small>演示导入后排队、本地转写与纪要。</small></label><label class="radio-option"><input type="radio" name="import-sample" value="invalid">不可用音频样例<small>体验不支持的输入提示，不创建会议。</small></label><p class="page-note">真实支持格式、大小限制、重复文件处理与双声源语义待用户确认。</p>', '导入虚构样例', () => {
      if ($('input[name="import-sample"]:checked').value === 'invalid') { $('#modal-error').textContent = '此样例格式不可用。请换用混音样例；没有创建会议或读取文件。'; return; }
      const title = $('#import-title').value.trim(); if (!title) { $('#modal-error').textContent = '请填写标题。'; return; }
      const m = createMeeting(title, '导入样例', false); save(); closeModal(); go(`#meeting/${m.id}`); toast('样例已导入并加入演示队列');
    });
  }
  function exportText(m, artifact, type) {
    const header = `# ${m.title}\n\n> 虚构演示内容 · ${m.date} ${m.time}${m.completedSegments < m.segments ? ' · 部分逐字稿，尚未完成' : ''}\n\n`;
    if (type === 'transcript') return header + artifact.transcript.map(t => `[${time(t.at)}] ${t.source === 'mic' ? '麦克风声源' : '系统声源'}：${t.text}`).join('\n\n');
    if (!artifact.summary) return header + '纪要尚未提交。\n';
    const s = artifact.summary;
    return header + s.intro + '\n\n## 关键决定\n' + s.decisions.map(t => '- ' + t).join('\n') + '\n\n## 讨论要点\n' + s.topics.map(t => `### ${t.title}\n${t.text}\n[${time(t.at)}]`).join('\n\n') + '\n\n## 行动项\n' + s.actions.map(a => `- ${a.text} · ${a.owner} · ${a.due}`).join('\n');
  }
  function download(text, filename, mime = 'text/plain;charset=utf-8') {
    try {
      const blob = new Blob([text], { type: mime }); const url = URL.createObjectURL(blob);
      const link = document.createElement('a'); link.href = url; link.download = filename; link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000); toast('已导出虚构演示内容');
    } catch { showModal('无法自动下载', `<p>浏览器限制了本地下载。可从下方复制演示内容。</p><div class="field"><label for="fallback-text">导出文本</label><textarea id="fallback-text" rows="10" readonly>${esc(text)}</textarea></div>`, null); }
  }
  function copyText(text) {
    // Manual selection also works on file:// when clipboard APIs are unavailable.
    showModal('复制演示文本', `<p>选中文本后按 ⌘C / Ctrl+C 复制。内容仅为虚构样例。</p><div class="field"><label for="copy-text">可复制内容</label><textarea id="copy-text" rows="10" readonly autofocus>${esc(text)}</textarea></div>`, null);
    $('#copy-text').select();
  }
  function syncPlayer() {
    const m = meeting(player.id); if (!m) return;
    player.position = Math.min(m.duration, Math.max(0, player.position));
    if ($('#audio-progress')) $('#audio-progress').value = player.position;
    if ($('#audio-time')) $('#audio-time').textContent = `${time(player.position)} / ${time(m.duration)}`;
    const play = $('[data-action="play"]'); if (play) { play.textContent = player.playing ? 'Ⅱ' : '▶'; play.setAttribute('aria-label', player.playing ? '暂停模拟播放' : '开始模拟播放'); }
    const lines = [...document.querySelectorAll('[data-line-at]')];
    let current = null; for (const line of lines) if (Number(line.dataset.lineAt) <= player.position) current = line;
    lines.forEach(line => line.classList.toggle('current', line === current));
  }
  function seek(at, locate) {
    const m = meeting(route.id); if (!m?.audio) return;
    player.id = m.id; player.position = at;
    if (locate) { detailTab = 'transcript'; transcriptQuery = ''; render(); }
    syncPlayer();
    if (locate) document.querySelector(`[data-line-at="${at}"]`)?.scrollIntoView({ behavior: 'smooth', block: 'center' });
  }
  setInterval(() => {
    if (!player.playing || route.page !== 'meeting') return;
    const m = meeting(player.id); if (!m) { player.playing = false; return; }
    player.position += player.speed * .5;
    if (player.position >= m.duration) { player.position = m.duration; player.playing = false; }
    syncPlayer();
  }, 500);
  const infoModals = {
    'service-info': ['服务生命周期', '<p>已有 CLI 支持用户级 install/start/status/stop/uninstall。停止或卸载需先成功封存录音并确认 helper 退出，卸载保留会议、模型和配置。</p><p>本演示不提供真实启动/停止按钮。网页服务控制权限和访问安全需在开发前确认。</p>'],
    'permission-info': ['录音授权指引', '<p>真实 OMM 需要麦克风、系统/屏幕音频等授权，窗口观察还可能需要辅助功能权限。权限请求只引导用户打开系统设置，不会自动授予。</p><p>本演示没有检测权限，也不会打开系统设置。录音可能包含系统其他应用声音，请在真实录制前告知参与者。</p>'],
    'model-info': ['固定依赖，不自动下载', '<p>本地转写：sherpa-onnx 1.13.2 C API、FireRedASR INT8 与 Silero VAD。编码：固定 FFmpeg 8.1.2 / libopus 1.5.2。</p><p>已有 CLI 可显式下载并校验模型、安装编码工具；模型在用时拒绝删除。这里不下载、安装或读取本机文件。</p><p>官方 ASR 库依赖 macOS 15.5+；不能由录音 helper 的 macOS 13 目标推定整套兼容。</p>'],
    'pi-info': ['专用 Pi 认证', '<p>真实 OMM 使用专用 CONFIG/omm/pi，不继承个人工具、扩展、上下文或凭据。认证通过 Pi 支持的方式完成，密钥不进入 OMM config.json 或浏览器本地存储。</p><p>入口路径、Node 和认证可能随升级失效；认证未知不是就绪。纪要失败只恢复纪要，不重复转写，不自动切模型。</p>'],
    'storage-info': ['存储与保留规则', '<p>音频、转写和纪要属于 DATA；任务与恢复记录属于 STATE；配置和专用 Pi 目录属于 CONFIG；可重建临时目录属于 CACHE。</p><p>历史录音不自动清理。新临时 WAV 只有在归档、检查点和读者保护验证完成后才可清理；不要删除未完成任务的检查点来“修复”失败。</p><p>macOS 替代运行目录采用最佳努力清理，不保证完整登录生命周期。这里仅展示说明，不管理文件。</p>'],
    'notification-preview': ['通知卡片预览', '<p>这只是页面内的卡片，不调用浏览器或桌面通知。</p><div class="panel wide-panel"><small>系统脚本宿主 · 预览</small><h3>Open Meeting Memo</h3><p>纸舟阅读计划 · 纪要已生成<br>10 月 16 日 14:00 · 31 分钟</p></div><p class="page-note">真实通知不含正文或凭据；调用结果和实际可见性分别验证。</p>']
  };
  document.addEventListener('click', event => {
    if (event.target.closest('.skip')) { event.preventDefault(); $('#main').focus(); return; }
    const button = event.target.closest('[data-action]');
    if (button) {
      const action = button.dataset.action; const m = meeting(button.dataset.id || route.id);
      if (infoModals[action]) { showModal(...infoModals[action], null); return; }
      switch (action) {
        case 'open-meeting': go(`#meeting/${button.dataset.id}`); break;
        case 'goto-queue': go('#queue'); break;
        case 'date-preset': Object.assign(filters, D.dateRange(button.dataset.preset), { preset: button.dataset.preset }); render(); $(`[data-preset="${filters.preset}"]`)?.focus({ preventScroll: true }); break;
        case 'clear-filters': filters = { ...defaultFilters(), from: '', to: '', preset: 'all' }; render(); $('#meeting-query')?.focus(); break;
        case 'unselect': selected.clear(); refreshLibraryResults(); break;
        case 'batch-delete': confirmDelete([...selected]); break;
        case 'delete': confirmDelete([m.id]); break;
        case 'edit': editMeeting(m); break;
        case 'resummary': requestProcess([m.id], 'summary'); break;
        case 'retranscribe': requestProcess([m.id], 'transcribe'); break;
        case 'batch-summary': requestProcess([...selected], 'summary'); break;
        case 'retry': retry(m); break;
        case 'advance': advance(m); save(); render(); toast('演示进度已推进一步'); break;
        case 'advance-all': state.meetings.forEach(advance); save(); render(); toast('所有在途演示任务已推进一步'); break;
        case 'inject-failure': m.status = 'failed'; m.error = '演示故障：处理超时。音频、检查点和旧结果保留，可重试未完成阶段。'; m.versions.at(-1).status = 'failed'; save(); render(); break;
        case 'cancel-task': showModal('取消这次演示任务？', `<p>${esc(m.title)}</p><p>停止本次模拟进度，保留音频与已提交结果。现有 CLI 尚无任务取消命令；真实开发需等待自有进程退出后再清理工作空间，不能把等待超时当作取消。</p>`, '确认取消任务', () => { m.status = 'cancelled'; m.error = null; m.versions.at(-1).status = 'cancelled'; save(); closeModal(); render(); toast('演示任务已取消，已有结果保留'); }, true); break;
        case 'task-details': showModal('处理详情', `<p>${esc(m.title)}</p><dl class="definition"><div><dt>当前阶段</dt><dd>${m.completedSegments < m.segments ? '本地转写' : 'Pi 纪要'}</dd></div><div><dt>已提交转写片段</dt><dd>${m.completedSegments} / ${m.segments}</dd></div><div><dt>本次重处理片段进度</dt><dd>${m.pendingSegments ?? m.completedSegments} / ${m.segments}</dd></div><div><dt>处理次数</dt><dd>${m.versions.length}</dd></div><div><dt>配置快照</dt><dd>${esc(m.versions.at(-1).model || '初始演示配置')}</dd></div></dl><p>数字仅为演示。失败不删除音频、不回显原始模型错误；重试保留有效检查点。</p>`, '打开关联会议', () => { closeModal(); go(`#meeting/${m.id}`); }); break;
        case 'clear-queue': queueFilter = ''; render(); break;
        case 'show-transcript': detailTab = 'transcript'; render(); break;
        case 'locate': seek(Number(button.dataset.at), true); break;
        case 'seek': seek(Number(button.dataset.at), false); break;
        case 'clear-transcript': transcriptQuery = ''; render(); $('#transcript-query')?.focus(); break;
        case 'play': if (player.position >= m.duration) player.position = 0; player.playing = !player.playing; player.id = m.id; syncPlayer(); break;
        case 'back-15': seek(player.position - 15, false); break;
        case 'forward-15': seek(player.position + 15, false); break;
        case 'read-version': selectedVersion = Number(button.dataset.version); detailTab = 'summary'; render(); break;
        case 'latest-version': selectedVersion = null; render(); break;
        case 'export': download(exportText(m, currentArtifact(m), button.dataset.artifact), `omm-demo-${button.dataset.artifact}.md`, 'text/markdown;charset=utf-8'); break;
        case 'copy': copyText(exportText(m, currentArtifact(m), button.dataset.artifact)); break;
        case 'batch-export': {
          const items = [...selected].map(meeting).filter(Boolean);
          showModal('导出所选会议', `<p>导出 ${items.length} 场虚构会议的当前已提交纪要与逐字稿。部分逐字稿将明确标记；没有内容的会议只导出基本信息。不包含录音文件或任何真实资料。</p>`, '导出演示 Markdown', () => { download(items.map(item => exportText(item, item, 'summary') + '\n\n' + exportText(item, item, 'transcript')).join('\n\n---\n\n'), 'omm-demo-meetings.md', 'text/markdown;charset=utf-8'); closeModal(); }); break;
        }
        case 'import': importModal(); break;
        case 'record': recordingModal(); break;
        case 'stop-record': {
          const recording = meeting(state.recording); if (!recording) break;
          showModal('停止并封存模拟录制', `<p>${esc(recording.title)}</p><p>封存一段 2 分钟的虚构样例并进入演示处理队列。没有真实录音。真实手动停止也会抑制当前会议的自动重启。</p>`, '停止并加入队列', () => {
            recording.status = 'queued'; recording.audio = true; recording.duration = 120; recording.segments = 1;
            recording.versions = [{ name: '初次处理', status: 'queued', stage: 'all' }]; state.recording = null;
            save(); closeModal(); render(); toast('模拟录制已封存并加入队列');
          }); break;
        }
        case 'discard-settings': showModal('放弃这次设置修改？', '<p>恢复到上次保存的演示参数。独立确认的自动录音策略不受影响。</p>', '放弃修改', () => { draft = clone(state.config); draftEvents = clone(state.events); errors = {}; closeModal(); render(); }); break;
        case 'toggle-auto':
          if (state.auto) { state.auto = false; save(); render(); toast('自动录音演示策略已关闭'); }
          else showModal('开启自动录音演示策略', '<p>真实自动录音会在强信号确认后采集麦克风和系统声音，系统轨可能包含其他应用。确认/轮询可能漏掉开头或延长尾部，窗口/进程不能单独起录。</p><label class="inline-label"><input id="auto-consent" type="checkbox" required>我已了解录音范围；此处仅保存演示策略。</label><p class="page-note">这不是实际录音授权，不请求权限，也不会监听会议。</p>', '确认开启演示策略', () => { if (!$('#auto-consent').checked) return; state.auto = true; state.consent = true; save(); closeModal(); render(); toast('演示策略已开启，不监听真实会议'); });
          break;
        case 'diagnostic-report': download(JSON.stringify({ demo: true, connected: false, permissions: 'unknown', models: 'not_checked', authentication: 'not_checked', notificationVisibility: 'unknown', fictionalMeetingCount: state.meetings.length }, null, 2), 'omm-demo-diagnostics.json', 'application/json'); break;
        case 'reset': showModal('恢复整套演示数据？', '<p>清除本浏览器中对虚构会议、设置和队列的修改，恢复最初六场样例。不会影响任何真实会议或 OMM 配置。</p>', '恢复演示数据', () => { state = D.fresh(); draft = clone(state.config); draftEvents = clone(state.events); errors = {}; selected.clear(); filters = defaultFilters(); player.playing = false; save(); closeModal(); if (route.page === 'library') render(); else navigate({ page: 'library', id: null }); toast('已恢复初始演示数据'); }, true); break;
        case 'close-modal': closeModal(); break;
      }
      return;
    }
    const tab = event.target.closest('[data-tab]');
    if (tab) { detailTab = tab.dataset.tab; render(); $(`#tab-${detailTab}`).focus({ preventScroll: true }); syncPlayer(); }
    const group = event.target.closest('[data-settings-group]');
    if (group) { settingsGroup = group.dataset.settingsGroup; render(); $(`[data-settings-group="${settingsGroup}"]`).focus({ preventScroll: true }); }
  });
  document.addEventListener('keydown', event => {
    const tab = event.target.closest('[data-tab]');
    if (!tab || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault(); const keys = ['summary', 'transcript', 'info'];
    const index = keys.indexOf(detailTab);
    detailTab = event.key === 'Home' ? keys[0] : event.key === 'End' ? keys.at(-1) : keys[(index + (event.key === 'ArrowRight' ? 1 : 2)) % 3];
    render(); $(`#tab-${detailTab}`).focus(); syncPlayer();
  });
  document.addEventListener('input', event => {
    const el = event.target;
    if (el.id === 'meeting-query') { filters.query = el.value; refreshLibraryResults(); }
    if (el.id === 'transcript-query') {
      transcriptQuery = el.value; const m = meeting(route.id); const artifact = currentArtifact(m);
      mount($('#transcript-lines'), transcriptLines(artifact));
      $('#transcript-count').textContent = `${artifact.transcript.filter(t => t.text.toLowerCase().includes(transcriptQuery.trim().toLowerCase())).length} 个片段`;
    }
    if (el.id === 'audio-progress') { player.position = Number(el.value); syncPlayer(); }
    if (el.dataset.setting) {
      const f = D.fields.find(field => field.key === el.dataset.setting);
      draft[f.key] = f.type === 'checkbox' ? el.checked : f.type === 'number' || f.options ? (el.value === '' ? '' : Number(el.value)) : el.value;
      const error = fieldError(f, draft[f.key]);
      if (error) errors[f.key] = error; else delete errors[f.key];
      el.setAttribute('aria-invalid', String(!!error)); $(`#${el.id}-error`).textContent = error; updateDirtyUi();
    }
  });
  document.addEventListener('change', event => {
    const el = event.target;
    const filterMap = { 'search-scope': 'scope', 'filter-source': 'source', 'filter-status': 'status', 'filter-from': 'from', 'filter-to': 'to', sort: 'sort' };
    if (filterMap[el.id]) { filters[filterMap[el.id]] = el.value; if (['filter-from', 'filter-to'].includes(el.id)) filters.preset = filters.from || filters.to ? 'custom' : 'all'; render(); $(`#${el.id}`)?.focus({ preventScroll: true }); }
    if (el.id === 'select-all') { filteredMeetings().forEach(m => el.checked ? selected.add(m.id) : selected.delete(m.id)); refreshLibraryResults(); }
    if (el.dataset.select) { if (el.checked) selected.add(el.dataset.select); else selected.delete(el.dataset.select); refreshLibraryResults(); $(`[data-select="${el.dataset.select}"]`)?.focus({ preventScroll: true }); }
    if (el.id === 'queue-filter') { queueFilter = el.value; render(); $('#queue-filter').focus(); }
    if (el.id === 'audio-speed') player.speed = Number(el.value);
    if (el.dataset.notificationEvent) { const key = el.dataset.notificationEvent; draftEvents = D.events.map(e => e[0]).filter(e => e === key ? el.checked : draftEvents.includes(e)); updateDirtyUi(); }
  });
  document.addEventListener('submit', event => {
    if (event.target.id === 'modal-form') { event.preventDefault(); modalSubmit?.(); }
    if (event.target.id === 'settings-form') {
      event.preventDefault();
      if (!validateDraft()) {
        const first = D.fields.find(f => errors[f.key]); settingsGroup = first.group; render(); $('[aria-invalid="true"]')?.focus(); toast('请先修正标记的设置'); return;
      }
      state.config = clone(draft); state.events = clone(draftEvents); save(); render(); toast('演示设置已保存；已有任务快照保持不变');
    }
  });
  // Pure seams for source-named regression tests, not backend integration hooks.
  window.DemoTest = { fresh: D.fresh, validState, filteredMeetings, fieldError, advance, enqueue, exportText, getState: () => clone(state) };
  route = parseHash();
  if (route.page === 'meeting') player.id = route.id;
  history.replaceState(null, '', route.page === 'meeting' ? `#meeting/${route.id}` : `#${route.page}`);
  render();
})();
