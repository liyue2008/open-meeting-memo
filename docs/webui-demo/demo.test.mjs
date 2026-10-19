// Run: node docs/webui-demo/demo.test.mjs
// Uses ONLY an existing Chrome in an isolated temporary profile. No packages installed.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { once } from 'node:events';

const root = path.dirname(fileURLToPath(import.meta.url));
const chrome = process.env.CHROME_BINARY || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const output = process.env.DEMO_TEST_OUTPUT || await mkdtemp(path.join(tmpdir(), 'omm-webui-evidence-'));
const profile = await mkdtemp(path.join(tmpdir(), 'omm-webui-chrome-'));
let child, ws;
let checks = 0;
const failures = [];
const requests = [];
const browserErrors = [];
function check(value, message) { assert.ok(value, message); checks++; }
try {
  for (const file of ['index.html', 'palette.css', 'styles.css', 'theme.js', 'sidebar.js', 'demo-data.js', 'app.js']) {
    const source = await readFile(path.join(root, file), 'utf8');
    check(!/https?:\/\//.test(source), `${file}: no external URL`);
    check(!/\b(fetch\s*\(|XMLHttpRequest|new\s+(?:Audio|WebSocket)|speechSynthesis|navigator\.mediaDevices|new\s+Notification)/.test(source), `${file}: no network/audio/notification APIs`);
  }
  child = spawn(chrome, ['--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-component-update', '--disable-sync', '--disable-default-apps', '--disable-extensions', '--metrics-recording-only', '--safebrowsing-disable-auto-update', '--mute-audio', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
  let stderr = '';
  const endpoint = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Chrome debugging endpoint timeout')), 15000);
    child.on('error', reject);
    child.stderr.on('data', data => { stderr += data; const match = stderr.match(/DevTools listening on (ws:\/\/\S+)/); if (match) { clearTimeout(timer); resolve(match[1]); } });
    child.on('exit', code => { if (!stderr.includes('DevTools listening')) reject(new Error(`Chrome exited ${code}: ${stderr}`)); });
  });
  ws = new WebSocket(endpoint);
  await once(ws, 'open');
  const pending = new Map(); let seq = 0;
  ws.addEventListener('message', event => {
    const data = JSON.parse(event.data);
    if (data.id) { const handler = pending.get(data.id); if (handler) { pending.delete(data.id); data.error ? handler.reject(new Error(JSON.stringify(data.error))) : handler.resolve(data.result); } }
    if (data.method === 'Runtime.exceptionThrown') browserErrors.push(data.params.exceptionDetails.exception?.description || data.params.exceptionDetails.text);
    if (data.method === 'Log.entryAdded' && data.params.entry.level === 'error') browserErrors.push(data.params.entry.text);
    if (data.method === 'Network.requestWillBeSent') requests.push(data.params.request.url);
    if (data.method === 'Fetch.requestPaused') send('Fetch.failRequest', { requestId: data.params.requestId, errorReason: 'BlockedByClient' }, data.sessionId);
  });
  function send(method, params = {}, sessionId) {
    const id = ++seq;
    return new Promise((resolve, reject) => { pending.set(id, { resolve, reject }); ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) })); });
  }
  const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  const cmd = (method, params = {}) => send(method, params, sessionId);
  await send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: output });
  await cmd('Runtime.enable'); await cmd('Page.enable'); await cmd('Log.enable'); await cmd('Network.enable');
  await cmd('Fetch.enable', { patterns: [{ urlPattern: 'http://*' }, { urlPattern: 'https://*' }] });
  const evaluate = async expression => {
    const result = await cmd('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true, userGesture: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    return result.result.value;
  };
  const wait = async expression => {
    for (let i = 0; i < 80; i++) { if (await evaluate(expression)) return; await new Promise(r => setTimeout(r, 35)); }
    throw new Error(`Timed out: ${expression}`);
  };
  const click = async selector => { check(await evaluate(`!!document.querySelector(${JSON.stringify(selector)})`), `exists: ${selector}`); await evaluate(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.focus();e.click();})()`); };
  const input = async (selector, value, type = 'input') => evaluate(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.value=${JSON.stringify(value)};e.dispatchEvent(new Event(${JSON.stringify(type)},{bubbles:true}));})()`);
  const nav = async hash => { await evaluate(`location.hash=${JSON.stringify(hash)}`); await wait(`location.hash===${JSON.stringify(hash)} && document.querySelector('#main h1')`); await new Promise(r => setTimeout(r, 60)); };
  const key = async key => { await cmd('Input.dispatchKeyEvent', { type: 'keyDown', key, code:key, windowsVirtualKeyCode:({Escape:27,ArrowRight:39,Home:36})[key] }); await cmd('Input.dispatchKeyEvent', { type: 'keyUp', key, code:key, windowsVirtualKeyCode:({Escape:27,ArrowRight:39,Home:36})[key] }); };
  const downloaded = async filename => {
    for (let i=0;i<60;i++) { try { return await readFile(path.join(output, filename), 'utf8'); } catch { await new Promise(r=>setTimeout(r,50)); } }
    throw new Error(`Download timeout: ${filename}`);
  };
  const text = () => evaluate('document.querySelector("#main").innerText');
  const confirm = () => click('#modal [type=submit]');
  const reset = async () => { await click('.sidebar [data-action=reset]'); await confirm(); await wait('location.hash==="#library"'); };
  const snap = async name => { const shot = await cmd('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true }); await writeFile(path.join(output, name + '.png'), Buffer.from(shot.data, 'base64')); };
  await cmd('Emulation.setDeviceMetricsOverride', { width: 1440, height: 1050, deviceScaleFactor: 1, mobile: false });
  await cmd('Page.navigate', { url: pathToFileURL(path.join(root, 'index.html')).href });
  await wait('!!window.DemoTest'); await reset();
  check(await evaluate('document.documentElement.dataset.sidebar==="expanded" && localStorage.getItem("omm-webui-sidebar-v1")===null'), 'sidebar defaults expanded without writing a preference');
  check((await evaluate('DemoTest.getState().meetings.length')) === 6, 'six fictional fixtures');
  check(await evaluate(`document.querySelector('[data-preset="seven"]').getAttribute('aria-pressed')==='true' && document.querySelectorAll('tbody tr').length===6`), 'reset defaults to recent seven days with usable fixtures');
  check(await evaluate(`DemoTest.getState().anchorDate===DemoData.localDay() && JSON.parse(localStorage.getItem('omm-webui-fictional-v1')).anchorDate===DemoData.localDay()`), 'generation anchor visibly uses persisted local calendar day');
  for (const [zone, utcDay] of [['Pacific/Kiritimati', '2026-01-01'], ['America/Los_Angeles', '2025-12-31']]) {
    await cmd('Emulation.setTimezoneOverride', { timezoneId: zone });
    check(await evaluate(`DemoData.localDay(new Date('2026-01-01T00:30:00Z'))===${JSON.stringify(utcDay)} && DemoData.localDay(new Date(2026,0,1,0,5))==='2026-01-01'`), `${zone}: local dates do not drift through UTC serialization`);
    for (const [preset, date, expected] of [
      ['week', [2026,0,4], ['2025-12-29','2026-01-04']], ['week',[2026,0,5],['2026-01-05','2026-01-05']],
      ['three',[2026,2,1],['2026-02-27','2026-03-01']], ['seven',[2026,0,3],['2025-12-28','2026-01-03']],
      ['seven',[2026,2,10],['2026-03-04','2026-03-10']], ['today',[2026,0,1],['2026-01-01','2026-01-01']]
    ]) check(await evaluate(`JSON.stringify(Object.values(DemoData.dateRange(${JSON.stringify(preset)},new Date(${date.join(',')},0,5))))===${JSON.stringify(JSON.stringify(expected))}`), `${zone}: ${preset} calendar boundary ${date}`);
  }
  await cmd('Emulation.setTimezoneOverride', { timezoneId: '' });
  for (const [preset, count] of [['today',1],['week',null],['three',3],['seven',6]]) {
    await click(`[data-preset="${preset}"]`);
    check(await evaluate(`(()=>{const r=DemoData.dateRange('${preset}');return document.querySelector('#filter-from').value===r.from&&document.querySelector('#filter-to').value===r.to&&document.querySelector('[data-preset="${preset}"]').getAttribute('aria-pressed')==='true';})()`), `${preset}: selected shortcut equals displayed range`);
    check(await evaluate(`DemoTest.filteredMeetings().every(m=>m.date>=document.querySelector('#filter-from').value&&m.date<=document.querySelector('#filter-to').value)`), `${preset}: date bounds inclusive`);
    if (count !== null) check((await evaluate('document.querySelectorAll("tbody tr").length')) === count, `${preset}: expected sample count`);
  }
  // Advance a synthetic local clock across Sunday midnight without waiting or reloading.
  await cmd('Emulation.setTimezoneOverride', { timezoneId: 'Asia/Shanghai' });
  await evaluate(`(()=>{const RealDate=Date;window.midnightTestClock={RealDate,now:0};window.Date=class extends RealDate{constructor(...args){super(...(args.length?args:[window.midnightTestClock.now]));}static now(){return window.midnightTestClock.now;}};})()`);
  const setMidnightClock = day => evaluate(`window.midnightTestClock.now=new midnightTestClock.RealDate(2026,9,${day},${day === 18 ? '23,59,30' : '0,0,30'}).getTime()`);
  try {
    for (const [preset, beforeFrom, afterFrom, beforeIds, afterIds] of [
      ['today','2026-10-18','2026-10-19',['garden'],[]],
      ['week','2026-10-12','2026-10-19',['garden','paper','island','trail','blank','radio'],[]],
      ['three','2026-10-16','2026-10-17',['garden','paper','island'],['garden','paper']],
      ['seven','2026-10-12','2026-10-13',['garden','paper','island','trail','blank','radio'],['garden','paper','island','trail','blank','radio']]
    ]) {
      await setMidnightClock(18); await reset(); await click(`[data-preset="${preset}"]`);
      check(await evaluate(`document.querySelector('#filter-from').value===${JSON.stringify(beforeFrom)}&&document.querySelector('#filter-to').value==='2026-10-18'&&JSON.stringify([...document.querySelectorAll('tbody .meeting-link')].map(e=>e.dataset.id))===${JSON.stringify(JSON.stringify(beforeIds))}`), `${preset}: pre-midnight range and actual results`);
      await setMidnightClock(19); await input('#meeting-query', '·');
      check(await evaluate(`JSON.stringify([...document.querySelectorAll('tbody .meeting-link')].map(e=>e.dataset.id))===${JSON.stringify(JSON.stringify(afterIds))}`), `${preset}: search after local midnight uses the new day`);
      check(await evaluate(`document.querySelector('#filter-from').value===${JSON.stringify(afterFrom)}&&document.querySelector('#filter-to').value==='2026-10-19'&&document.querySelectorAll('[data-preset][aria-pressed="true"]').length===1&&document.querySelector('[data-preset="${preset}"]').getAttribute('aria-pressed')==='true'`), `${preset}: after-midnight inputs and selected shortcut agree with query`);
    }
    await nav('#queue'); await nav('#library');
    check(await evaluate(`document.querySelector('#filter-from').value==='2026-10-13'&&document.querySelector('#filter-to').value==='2026-10-19'&&document.querySelector('[data-preset="seven"]').getAttribute('aria-pressed')==='true'`), 'rolling shortcut survives page return after midnight');
    await setMidnightClock(18); await reset();
    await input('#filter-from', '2026-10-16', 'change'); await input('#filter-to', '2026-10-18', 'change');
    await setMidnightClock(19); await input('#meeting-query', '·');
    check(await evaluate(`document.querySelector('#filter-from').value==='2026-10-16'&&document.querySelector('#filter-to').value==='2026-10-18'&&!document.querySelector('[data-preset][aria-pressed="true"]')&&JSON.stringify([...document.querySelectorAll('tbody .meeting-link')].map(e=>e.dataset.id))==='["garden","paper","island"]'`), 'custom dates and actual results do not roll across midnight on search');
    await nav('#queue'); await nav('#library');
    check(await evaluate(`document.querySelector('#filter-from').value==='2026-10-16'&&document.querySelector('#filter-to').value==='2026-10-18'&&!document.querySelector('[data-preset][aria-pressed="true"]')`), 'custom dates remain unchanged on page return after midnight');
  } finally {
    await evaluate('window.Date=midnightTestClock.RealDate;delete window.midnightTestClock');
    await cmd('Emulation.setTimezoneOverride', { timezoneId: '' });
    await reset();
  }
  await nav('#queue'); await nav('#library');
  check(await evaluate(`document.querySelector('[data-preset="seven"]').getAttribute('aria-pressed')==='true'`), 'return to library keeps chosen date filter');
  check(await evaluate('DemoTest.validState(DemoTest.fresh())'), 'fresh state valid');
  check(!(await evaluate('DemoTest.validState({schema:1,meetings:[]})')), 'corrupt state rejected');
  check(await evaluate('DemoTest.getState().meetings.every(m=>m.transcript.every(t=>t.at<=m.duration))'), 'fixture timestamps fit duration');
  await new Promise(r => setTimeout(r, 3300)); await snap('desktop-library');
  check(!(await evaluate('DemoTest.validState({...DemoTest.fresh(),meetings:[null]})')), 'null meeting rejected');
  check(!(await evaluate('(()=>{const s=DemoTest.fresh();s.meetings[0].summary.actions=[null];return DemoTest.validState(s);})()')), 'malformed nested summary rejected');
  await input('#filter-from', '2026-10-20', 'change'); await input('#filter-to', '2026-10-10', 'change');
  check((await text()).includes('开始日期晚于结束日期'), 'invalid date range explained'); await click('[data-action=clear-filters]');
  await input('#meeting-query', '不存在的词'); check((await text()).includes('没有找到匹配'), 'no-results state');
  await click('[data-action=clear-filters]'); await input('#meeting-query', '纸舟');
  check((await evaluate('document.querySelectorAll("tbody tr").length')) === 1, 'title search');
  await click('[data-action=clear-filters]'); await input('#filter-source', '手动录制', 'change');
  check((await evaluate('document.querySelectorAll("tbody tr").length')) === 2, 'source filter');
  await input('#filter-status', 'failed', 'change'); check((await evaluate('document.querySelectorAll("tbody tr").length')) === 1, 'combined status filter');
  await click('[data-action=clear-filters]');
  const anchor = await evaluate('DemoTest.getState().anchorDate');
  const previousDay = async offset => evaluate(`DemoData.shiftDay(${JSON.stringify(anchor)},${offset})`);
  await input('#filter-from', await previousDay(-2), 'change'); await input('#filter-to', await previousDay(-1), 'change');
  check(await evaluate(`!document.querySelector('[data-preset][aria-pressed="true"]') && document.querySelector('.date-shortcuts').innerText.includes('自定义日期')`), 'manual range clears shortcut selection');
  check((await evaluate('document.querySelectorAll("tbody tr").length')) === 2, 'date interval filter');
  await input('#sort', 'oldest', 'change'); check((await evaluate('document.querySelector("tbody .meeting-link").textContent')).includes('小岛'), 'sort oldest');
  await click('[data-action=clear-filters]'); await input('#search-scope', 'content', 'change'); await input('#meeting-query', '封面');
  check((await evaluate('document.querySelectorAll("tbody tr").length')) >= 1, 'fictional content search');
  for (const [field, keyword] of [['topic title', '先验证核心路径'], ['topic text', '连续打卡'], ['action text', '输出书单'], ['action owner', '许青'], ['action due', '10 月 21 日']]) {
    check(await evaluate(`(()=>{const m=DemoTest.fresh().meetings[0];return ![m.title,...m.transcript.map(t=>t.text),m.summary.intro,...m.summary.decisions].join(' ').includes(${JSON.stringify(keyword)});})()`), `${field}: keyword exists only in newly covered fields`);
    await input('#meeting-query', keyword);
    check(await evaluate('!!document.querySelector("[data-action=open-meeting][data-id=paper]")'), `content search includes ${field}`);
  }
  await click('[data-action=clear-filters]'); await click('#select-all'); check((await text()).includes('已选 6 场'), 'select all');
  await click('[data-action=batch-delete]'); check((await evaluate('document.querySelector("#modal").innerText')).includes('先处理在途工作'), 'busy deletion blocked'); await click('#modal [data-action=close-modal]');
  await click('[data-action=unselect]');
  await click('[data-select=paper]'); await click('[data-select=trail]'); await click('[data-action=batch-export]'); await confirm();
  const bulk = await downloaded('omm-demo-meetings.md'); check(bulk.includes('纸舟阅读') && bulk.includes('山径地图') && bulk.includes('虚构演示内容'), 'batch downloads only fictional text');
  await click('[data-action=batch-summary]'); await confirm(); await nav('#queue');
  for (let i=0;i<2;i++) { await click('[data-action=advance][data-id=paper]'); await click('[data-action=advance][data-id=trail]'); }
  check((await evaluate('DemoTest.getState().meetings.filter(m=>["paper","trail"].includes(m.id)).every(m=>m.status==="completed")')), 'batch summary completes');
  await reset();
  await nav('#meeting/paper'); check((await text()).includes('关键决定'), 'summary reading');
  await click('.skip'); check((await evaluate('location.hash'))==='#meeting/paper', 'skip link keeps route');
  check(await evaluate('document.activeElement.id==="main"'), 'skip link focuses main');
  await evaluate('document.querySelector("#tab-summary").focus()'); await key('ArrowRight'); check(await evaluate('document.activeElement.id==="tab-transcript"'), 'keyboard tab navigation');
  await key('Home'); check(await evaluate('document.activeElement.id==="tab-summary"'), 'keyboard tab Home');
  await snap('desktop-workspace');
  await click('[data-action=export][data-artifact=summary]'); check((await downloaded('omm-demo-summary.md')).includes('关键决定'), 'summary download');
  check(await evaluate(`document.querySelectorAll('.action-item').length===3 && !document.querySelector('.actions-list input,.action-item.done,[data-action-item]')`), 'actions retain text but have no tracking control or completion style');
  check(await evaluate(`!Object.hasOwn(DemoTest.getState().meetings[0].summary.actions[0],'done') && !DemoTest.exportText(DemoTest.getState().meetings[0],DemoTest.getState().meetings[0],'summary').includes('- [')`), 'fixtures and exports have no action completion state');
  check(await evaluate(`document.querySelector('.audio-panel').getBoundingClientRect().bottom<=document.querySelector('.tabs').getBoundingClientRect().top && document.querySelector('.audio-panel').getBoundingClientRect().top<innerHeight`), 'desktop player precedes tabs and document in first viewport');
  await click('[data-action=locate]'); check(await evaluate('document.querySelector("#tab-transcript").getAttribute("aria-selected")==="true"'), 'summary time locates transcript');
  check((await evaluate('Number(document.querySelector("#audio-progress").value)')) === 46, 'timestamp seeks player');
  await click('[data-action=export][data-artifact=transcript]'); check((await downloaded('omm-demo-transcript.md')).includes('[00:46]'), 'transcript download');
  await input('#transcript-query', '书单'); check((await evaluate('document.querySelectorAll("mark").length')) > 0, 'transcript search highlight');
  await input('#transcript-query', '<script>'); check((await text()).includes('没有匹配的片段'), 'search string inert');
  await click('[data-action=clear-transcript]'); await input('#audio-progress', 130); await input('#audio-speed', '2', 'change');
  await click('[data-action=play]'); await new Promise(r => setTimeout(r, 650));
  check((await evaluate('Number(document.querySelector("#audio-progress").value)')) > 130, 'silent player advances'); await click('[data-action=play]');
  await click('[data-action=forward-15]'); check((await evaluate('Number(document.querySelector("#audio-progress").value)')) > 145, 'jump forward');
  check(await evaluate('document.querySelectorAll("audio,video").length===0'), 'no media elements');
  await click('#tab-summary'); await click('[data-action=copy]'); check(await evaluate('document.activeElement.id==="copy-text"'), 'copy selects text accessibly'); await click('#modal [data-action=close-modal]');
  await click('[data-action=edit]'); await key('Escape'); await wait('!document.querySelector("#modal").open'); check(!(await evaluate('document.querySelector("#modal").open')), 'Escape closes modal');
  check(await evaluate('document.activeElement.dataset.action==="edit"'), 'modal returns focus');
  await click('[data-action=edit]'); await input('#edit-title', '<img src=x onerror=alert(1)> 纸舟演示'); await confirm();
  check((await text()).includes('<img src=x'), 'metadata displayed as text'); check(!(await evaluate('!!document.querySelector("#main img")')), 'metadata cannot inject markup');
  await click('[data-action=edit]'); await input('#edit-title', '纸舟阅读计划 · 第一版体验评审'); await confirm();
  await click('[data-action=resummary]'); await confirm();
  check((await text()).includes('关键决定'), 'prior result readable while reprocessing');
  await nav('#queue'); await click('[data-action=advance][data-id=paper]'); await click('[data-action=inject-failure][data-id=paper]');
  check((await evaluate('DemoTest.getState().meetings.find(m=>m.id==="paper").summary!==null')), 'failure preserves previous summary');
  await click('[data-action=retry][data-id=paper]'); await confirm(); await click('[data-action=advance][data-id=paper]'); await click('[data-action=advance][data-id=paper]');
  check((await evaluate('DemoTest.getState().meetings.find(m=>m.id==="paper").status')) === 'completed', 'retry completes summary');
  await nav('#meeting/paper'); await click('#tab-info'); await click('[data-action=read-version][data-version="0"]');
  check((await text()).includes('正在阅读历史版本'), 'historical result readable'); await click('[data-action=latest-version]');
  await click('[data-action=retranscribe]'); await confirm(); await nav('#queue'); await click('[data-action=cancel-task][data-id=paper]'); await confirm();
  check((await evaluate('DemoTest.getState().meetings.find(m=>m.id==="paper").status')) === 'cancelled', 'cancel static task');
  await snap('desktop-queue');
  // A full final without minutes (single-segment island fixture) is still a committed input.
  // Exercise both interruption paths through the UI, then successfully resume the same attempt.
  for (const outcome of ['failure', 'cancel']) {
    await reset();
    const original = await evaluate('DemoTest.getState().meetings.find(m=>m.id==="island")');
    const originalArtifact = JSON.stringify({ transcript: original.transcript, summary: original.summary });
    await nav('#meeting/island'); await click('[data-action=retranscribe]'); await confirm(); await nav('#queue');
    await click('[data-action=advance][data-id=island]'); await click('[data-action=advance][data-id=island]');
    let current = await evaluate('DemoTest.getState().meetings.find(m=>m.id==="island")');
    check(current.pendingSegments === 0, `${outcome}: single segment not prematurely marked committed`);
    check(JSON.stringify({ transcript: current.transcript, summary: current.summary }) === originalArtifact && current.completedSegments === original.completedSegments, `${outcome}: midway reprocessing keeps full final without minutes`);
    check(JSON.stringify(current.versions[0].snapshot) === originalArtifact, `${outcome}: midway original version unchanged`);
    await nav('#meeting/island'); await click('#tab-transcript');
    check((await evaluate('document.querySelectorAll(".transcript-line").length')) === original.transcript.length && !(await text()).includes('部分逐字稿'), `${outcome}: default reader retains complete original`);
    await nav('#queue');
    if (outcome === 'failure') await click('[data-action=inject-failure][data-id=island]');
    else { await click('[data-action=cancel-task][data-id=island]'); await confirm(); }
    current = await evaluate('DemoTest.getState().meetings.find(m=>m.id==="island")');
    check(current.status === (outcome === 'failure' ? 'failed' : 'cancelled'), `${outcome}: interrupted at expected state`);
    check(JSON.stringify({ transcript: current.transcript, summary: current.summary }) === originalArtifact && current.completedSegments === original.completedSegments, `${outcome}: interruption preserves final and minutes input`);
    check(JSON.stringify(current.versions[0].snapshot) === originalArtifact, `${outcome}: interruption preserves original snapshot`);
    await nav('#meeting/island'); await click('#tab-transcript');
    check((await evaluate('document.querySelectorAll(".transcript-line").length')) === original.transcript.length, `${outcome}: interrupted default reader still full`);
    check(await evaluate('!document.querySelector("[data-action=resummary]").disabled'), `${outcome}: full final remains eligible for minutes`);
    check(await evaluate(`(()=>{const m=DemoTest.getState().meetings.find(m=>m.id==='island');DemoTest.enqueue(m,'summary');DemoTest.advance(m);DemoTest.advance(m);return JSON.stringify(m.transcript)===${JSON.stringify(JSON.stringify(original.transcript))};})()`), `${outcome}: new minutes attempt reuses unchanged full final`);
    await click('[data-action=resummary]'); await click('#modal [data-action=close-modal]');
    await nav('#queue'); await click('[data-action=retry][data-id=island]'); await confirm();
    for (let i = 0; i < 3; i++) await click('[data-action=advance][data-id=island]');
    current = await evaluate('DemoTest.getState().meetings.find(m=>m.id==="island")');
    check(current.status === 'completed' && current.completedSegments === current.segments && current.transcript.length === 10 && !!current.summary, `${outcome}: resumed attempt commits new result`);
    check(JSON.stringify(current.versions[0].snapshot) === originalArtifact && current.activeVersion === 1, `${outcome}: successful commit never rewrites original snapshot`);
    await nav('#meeting/island'); await click('#tab-info'); await click('[data-action=read-version][data-version="0"]'); await click('#tab-transcript');
    check((await evaluate('document.querySelectorAll(".transcript-line").length')) === original.transcript.length, `${outcome}: original full final readable after success`);
  }
  await reset();
  await nav('#meeting/blank'); check((await text()).includes('没有可播放的录音'), 'empty recording state');
  await nav('#library'); await click('[data-action=import]'); await click('[value=invalid]'); await confirm(); check((await evaluate('document.querySelector("#modal-error").textContent')).includes('格式不可用'), 'invalid import shows error');
  await click('[value=normal]'); await confirm(); await wait('location.hash.startsWith("#meeting/sample-")');
  check((await evaluate('DemoTest.getState().meetings.length')) === 7, 'sample import creates meeting');
  await nav('#library'); await click('[data-action=record]'); await click('#record-consent'); await confirm();
  await wait('!!document.querySelector("[data-action=stop-record]")'); await click('[data-action=stop-record]'); await confirm();
  check((await evaluate('DemoTest.getState().recording')) === null, 'simulated recording seals');
  check((await evaluate('DemoTest.getState().meetings.length')) === 8, 'simulated recording creates meeting');
  await nav('#settings'); await input('#setting-segment_ms', 1); await click('#settings-form [type=submit]');
  check(await evaluate('document.activeElement.getAttribute("aria-invalid")==="true"'), 'invalid setting focused');
  await input('#setting-segment_ms', 60000); await click('[data-settings-group="Pi 纪要"]'); await input('#setting-pi-model', 'demo-model-revised');
  await evaluate('location.hash="#library"'); await wait('document.querySelector("#modal").open'); check((await evaluate('document.querySelector("#modal").innerText')).includes('还有未保存'), 'dirty navigation warning');
  await click('#modal [data-action=close-modal]'); check((await evaluate('location.hash')) === '#settings', 'cancel navigation stays');
  await click('#settings-form [type=submit]'); check((await evaluate('DemoTest.getState().config["pi.model"]')) === 'demo-model-revised', 'settings save');
  await click('[data-settings-group="通知"]'); await click('[data-notification-event=asr_failed]'); await click('[data-action=discard-settings]'); await confirm();
  check(await evaluate('document.querySelector("[data-notification-event=asr_failed]").checked'), 'discard settings restores event');
  await click('[data-settings-group="录音与检测"]'); await click('[data-action=toggle-auto]'); await click('#auto-consent'); await confirm();
  check(await evaluate('DemoTest.getState().auto && DemoTest.getState().consent'), 'auto strategy has separate consent');
  await snap('desktop-settings');
  await cmd('Page.reload'); await new Promise(r => setTimeout(r, 200)); await wait('!!window.DemoTest');
  check((await evaluate('DemoTest.getState().config["pi.model"]')) === 'demo-model-revised', 'reload persists fictional settings');
  // Exercise helpers against every fresh fixture without mutating UI state.
  check(await evaluate(`(()=>{const m=DemoTest.fresh().meetings[0];DemoTest.enqueue(m,'transcribe');for(let i=0;i<4;i++)DemoTest.advance(m);return m.status==='completed'&&m.versions.length===2&&!!m.versions[0].snapshot;})()`), 'transcription complete preserves history');
  check(await evaluate(`(()=>{const m=DemoTest.fresh().meetings[4];m.duration=120;m.audio=true;m.segments=1;DemoTest.enqueue(m,'transcribe');DemoTest.advance(m);DemoTest.advance(m);return m.pendingSegments===0&&m.completedSegments===0&&m.transcript.length===0;})()`), 'single-segment initial processing exposes no half-final before segment commit');
  check(await evaluate(`(()=>{const m=DemoTest.fresh().meetings[4];m.duration=120;m.audio=true;m.segments=1;DemoTest.enqueue(m,'transcribe');for(let i=0;i<4;i++)DemoTest.advance(m);return m.transcript.every(t=>t.at<=120)&&m.summary.topics.every(t=>t.at<=120);})()`), 'short sample timestamps scaled');
  await nav('#diagnostics'); check((await text()).includes('未连接真实 OMM'), 'diagnostic uncertainty explicit');
  await click('[data-action=notification-preview]'); check((await evaluate('document.querySelector("#modal").innerText')).includes('不调用浏览器或桌面通知'), 'page-only notification preview'); await click('#modal [data-action=close-modal]');
  await nav('#help'); check((await evaluate('document.querySelectorAll("details").length')) === 5, 'help FAQ rendered');
  // Narrow-screen layout and screenshots, including forms and reader.
  await cmd('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: false });
  for (const [hash, name] of [['#library', 'narrow-library'], ['#meeting/paper', 'narrow-workspace'], ['#settings', 'narrow-settings'], ['#queue', 'narrow-queue'], ['#help', 'narrow-help']]) {
    await nav(hash); check(await evaluate('document.documentElement.scrollWidth <= window.innerWidth'), `${name}: no horizontal overflow`);
    if (hash==='#meeting/paper') {
      check(await evaluate(`document.querySelector('.audio-panel').getBoundingClientRect().bottom<=document.querySelector('.tabs').getBoundingClientRect().top && document.querySelector('.audio-panel').getBoundingClientRect().top<innerHeight`), 'narrow player visible before reading tabs');
      check(await evaluate(`(()=>{const e=document.querySelector('#audio-time');const r=e.getBoundingClientRect();return r.width>0&&r.left>=0&&r.right<=innerWidth&&getComputedStyle(e).visibility!=='hidden'&&e.textContent.includes('31:00');})()`), 'narrow playback time remains visible within screen');
    }
    await snap(name);
  }
  await reset();
  // Delete a non-busy fixture, verify durable removal; then clear all library items using test-only state.
  await nav('#meeting/paper'); await click('[data-action=delete]'); check(await evaluate('document.activeElement.dataset.action==="close-modal"'), 'danger modal initially focuses cancel'); await confirm(); await wait('location.hash==="#library"');
  check((await evaluate('DemoTest.getState().meetings.length')) === 5, 'confirmed deletion');
  await reset();
  await evaluate('(()=>{const s=DemoTest.fresh();s.meetings=[];localStorage.setItem("omm-webui-fictional-v1",JSON.stringify(s));})()');
  await cmd('Page.reload'); await new Promise(r=>setTimeout(r,200)); await wait('!!window.DemoTest');
  check((await text()).includes('还没有会议'), 'empty library has creation entry'); await snap('narrow-empty-library'); await reset();
  // Older fictional stores migrate in place, retaining text/config but removing tracking state.
  await evaluate(`(()=>{const s=DemoTest.fresh();s.schema=1;delete s.anchorDate;s.meetings.forEach((m,i)=>m.date=['2026-10-16','2026-10-17','2026-10-15','2026-10-14','2026-10-13','2026-10-12'][i]);s.meetings[0].title='保留的虚构修改';s.meetings[0].summary.actions[0].done=true;s.meetings[0].versions[0].snapshot=JSON.parse(JSON.stringify(s.meetings[0]));localStorage.setItem('omm-webui-fictional-v1',JSON.stringify(s));})()`);
  await cmd('Page.reload'); await new Promise(r=>setTimeout(r,200)); await wait('!!window.DemoTest');
  check(await evaluate(`(()=>{const s=DemoTest.getState();return DemoTest.validState(s)&&s.schema===2&&s.meetings[0].title==='保留的虚构修改'&&s.meetings[0].date===DemoData.shiftDay(s.anchorDate,-1)&&!Object.hasOwn(s.meetings[0].summary.actions[0],'done')&&!Object.hasOwn(s.meetings[0].versions[0].snapshot.summary.actions[0],'done')&&JSON.parse(localStorage.getItem('omm-webui-fictional-v1')).schema===2;})()`), 'legacy dates re-anchor once; current and historical tracking removed; edits retained');
  await nav('#meeting/paper');
  check(await evaluate(`document.querySelectorAll('.action-item').length===3&&!document.querySelector('.actions-list input')`), 'legacy action status cannot reintroduce tracking');
  await nav('#library');
  // Move one fixture just outside the default window, then find it using custom dates.
  await evaluate(`(()=>{const s=DemoTest.getState();s.meetings[5].date=DemoData.shiftDay(DemoData.localDay(),-7);localStorage.setItem('omm-webui-fictional-v1',JSON.stringify(s));})()`);
  await cmd('Page.reload'); await new Promise(r=>setTimeout(r,200)); await wait('!!window.DemoTest');
  check((await evaluate('document.querySelectorAll("tbody tr").length'))===5, 'default seven-day window excludes day minus seven');
  await input('#filter-from', await previousDay(-7), 'change');
  check((await evaluate('document.querySelectorAll("tbody tr").length'))===6, 'custom range includes older meeting at exact lower boundary');
  await input('#filter-to', await previousDay(-7), 'change');
  check((await evaluate('document.querySelectorAll("tbody tr").length'))===1, 'custom upper/lower date boundaries both inclusive');
  await nav('#queue'); await nav('#library');
  check((await evaluate('document.querySelector("#filter-from").value'))===await previousDay(-7), 'custom range survives route changes');
  await click('[data-action=clear-filters]');
  check(await evaluate(`!document.querySelector('#filter-from').value&&!document.querySelector('#filter-to').value&&document.querySelectorAll('tbody tr').length===6&&document.querySelector('.date-shortcuts').innerText.includes('全部日期')`), 'clear filters means all dates, not a permanent seven-day limit');
  await reset();
  // Appearance-only changes must never rerender or mutate business state.
  const emulateTheme = async (scheme, reduced = 'no-preference') => cmd('Emulation.setEmulatedMedia', { features: [{name:'prefers-color-scheme',value:scheme},{name:'prefers-reduced-motion',value:reduced}] });
  const chooseTheme = value => input('#theme-preference', value, 'change');
  const effectiveTheme = () => evaluate('document.documentElement.dataset.theme');
  const reloadDemo = async () => { await cmd('Page.reload'); await new Promise(r=>setTimeout(r,200)); await wait('!!window.DemoTest'); };
  await emulateTheme('light'); await chooseTheme('system');
  check(await effectiveTheme()==='light', 'system mode follows emulated light OS');
  await emulateTheme('dark'); await wait('document.documentElement.dataset.theme==="dark"');
  check(await evaluate('getComputedStyle(document.documentElement).colorScheme==="dark"'), 'effective dark theme sets native color-scheme');
  await chooseTheme('light'); await emulateTheme('light'); await emulateTheme('dark');
  check(await effectiveTheme()==='light', 'manual light survives OS changes');
  await chooseTheme('dark'); await emulateTheme('light');
  check(await effectiveTheme()==='dark', 'manual dark survives OS light');
  await reloadDemo();
  check(await effectiveTheme()==='dark' && await evaluate('document.querySelector("#theme-preference").value==="dark"'), 'manual preference survives reload');
  await chooseTheme('system');
  check(await effectiveTheme()==='light', 'return to system synchronizes immediately');
  await reloadDemo(); check(await evaluate('document.documentElement.dataset.themePreference==="system"'), 'system preference survives reload');
  await input('#meeting-query','纸舟');
  const filteredBeforeTheme = await evaluate('document.querySelector("#meeting-query").value+":"+document.querySelectorAll("tbody tr").length');
  await chooseTheme('dark');
  check(await evaluate('document.querySelector("#meeting-query").value+":"+document.querySelectorAll("tbody tr").length')===filteredBeforeTheme, 'theme preserves library query and results');
  await nav('#meeting/paper'); await input('#audio-progress',123);
  const beforeThemeState = await evaluate('JSON.stringify(DemoTest.getState())');
  await click('[data-action=edit]'); await input('#edit-title','尚未保存的虚构标题'); await chooseTheme('light');
  check(await evaluate('document.querySelector("#modal").open && document.querySelector("#edit-title").value==="尚未保存的虚构标题"'), 'theme keeps modal and unsaved form input');
  await click('#modal [data-action=close-modal]');
  check(await evaluate('Number(document.querySelector("#audio-progress").value)===123 && location.hash==="#meeting/paper"'), 'theme preserves current meeting and silent playback position');
  check(await evaluate('JSON.stringify(DemoTest.getState())')===beforeThemeState, 'theme cannot mutate meetings, config, tasks or playback business state');
  await nav('#settings'); await input('#setting-segment_ms',60000); await chooseTheme('dark');
  check(await evaluate('document.querySelector("#setting-segment_ms").value==="60000" && document.querySelector("#save-state").textContent.includes("未保存")'), 'theme keeps unsaved settings draft');
  await click('[data-action=discard-settings]'); await confirm();
  // First animation frame must already have the correct saved/system appearance.
  const firstFrame = await cmd('Page.addScriptToEvaluateOnNewDocument', { source: 'requestAnimationFrame(()=>window.themeFirstFrame={theme:document.documentElement.dataset.theme,scheme:getComputedStyle(document.documentElement).colorScheme});' });
  for (const [preference, os, expected] of [['dark','light','dark'],['system','dark','dark'],['light','dark','light']]) {
    await chooseTheme(preference); await emulateTheme(os); await reloadDemo();
    check(await evaluate(`window.themeFirstFrame?.theme==='${expected}'&&window.themeFirstFrame?.scheme==='${expected}'`), `first frame already ${expected}: ${preference}/${os}`);
  }
  await cmd('Page.removeScriptToEvaluateOnNewDocument',{identifier:firstFrame.identifier});
  await evaluate('localStorage.setItem("omm-webui-theme-v1","broken")'); await reloadDemo();
  check(await evaluate('document.documentElement.dataset.themePreference==="system" && document.documentElement.dataset.theme==="dark" && !document.querySelector("#theme-warning").hidden'), 'corrupt theme falls back to current OS with visible explanation');
  await chooseTheme('light'); check(await evaluate('document.querySelector("#theme-warning").hidden && localStorage.getItem("omm-webui-theme-v1")==="light"'), 'choosing a valid theme repairs corrupt preference');
  const deniedWrite = await cmd('Page.addScriptToEvaluateOnNewDocument', { source: 'Storage.prototype.setItem=function(){throw Error("quota")};' });
  await reloadDemo(); await chooseTheme('dark');
  check(await effectiveTheme()==='dark' && await evaluate('!document.querySelector("#theme-warning").hidden'), 'theme write denied remains usable with visible warning');
  await cmd('Page.removeScriptToEvaluateOnNewDocument',{identifier:deniedWrite.identifier}); await reloadDemo();
  check(await effectiveTheme()==='light' && await evaluate('document.documentElement.dataset.themePreference==="light" && localStorage.getItem("omm-webui-theme-v1")==="light"'), 'reload after denied write restores previously saved light preference');
  await emulateTheme('light','reduce');
  check(await evaluate('getComputedStyle(document.querySelector("button")).transitionDuration==="0s"'), 'reduced motion disables transitions');
  const contrastSamples = [];
  const contrast = (a,b) => {
    const luminance = color => { const c=color.match(/[\d.]+/g).slice(0,3).map(Number).map(n=>{n/=255;return n<=.04045?n/12.92:((n+.055)/1.055)**2.4;});return c[0]*.2126+c[1]*.7152+c[2]*.0722; };
    const x=luminance(a), y=luminance(b); return (Math.max(x,y)+.05)/(Math.min(x,y)+.05);
  };
  const sampleContrast = async (label, selectors, minimum=4.5, property='color') => {
    for (const selector of selectors) {
      const sample = await evaluate(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e||!e.getClientRects().length||e.disabled)return null;let b=${property==='color'?'e':'e.parentElement'};while(b&&getComputedStyle(b).backgroundColor==='rgba(0, 0, 0, 0)')b=b.parentElement;return {fg:getComputedStyle(e)[${JSON.stringify(property)}],bg:b?getComputedStyle(b).backgroundColor:'rgb(255, 255, 255)'};})()`);
      if (!sample) continue;
      const ratio=contrast(sample.fg,sample.bg); contrastSamples.push({label,selector,...sample,ratio:Number(ratio.toFixed(3)),minimum});
      check(ratio>=minimum, `${label} ${selector} contrast ${ratio.toFixed(2)} >= ${minimum}`);
    }
  };
  const cleanSnap = async name => { await evaluate('document.activeElement?.blur()'); await cmd('Input.dispatchMouseEvent',{type:'mouseMoved',x:1,y:1}); await new Promise(r=>setTimeout(r,100)); await snap(name); };
  await nav('#library'); await reset(); await new Promise(r=>setTimeout(r,3300));
  // All routes, both themes and sizes, plus editing/reprocessing/destructive dialogs.
  for (const mode of ['light','dark']) {
    await chooseTheme(mode);
    if (mode === 'dark') {
      const dimmed = await evaluate('(()=>{const s=getComputedStyle(document.documentElement);return {background:s.backgroundColor,ink:s.color,surface:getComputedStyle(document.querySelector(".panel")).backgroundColor,accent:getComputedStyle(document.querySelector(".sidebar nav a.active .nav-icon")).color,muted:s.getPropertyValue("--muted").trim(),line:s.getPropertyValue("--line").trim(),control:s.getPropertyValue("--control-line").trim()};})()');
      const channels = color => color.match(/[\d.]+/g).slice(0,3).map(Number);
      const readingContrast = contrast(dimmed.ink, dimmed.background);
      check(channels(dimmed.background).every(value => value >= 30 && value <= 64), 'dimmed dark background is charcoal rather than near-black');
      check(dimmed.background==='rgb(37, 42, 47)' && dimmed.surface==='rgb(43, 48, 53)' && dimmed.ink==='rgb(201, 207, 211)' && dimmed.muted==='#a7b0b7', 'crisp accents preserve the approved reading background, surface, body and secondary text');
      check(Math.max(...channels(dimmed.ink)) <= 220 && readingContrast >= 4.5 && readingContrast <= 10, 'dimmed dark body text stays readable without near-white glare');
      check(Math.max(...channels(dimmed.accent))-Math.min(...channels(dimmed.accent)) >= 120, 'small dark teal accents are vivid rather than gray-muted');
      await writeFile(path.join(output,'soft-dark-colors.json'),JSON.stringify({...dimmed,readingContrast},null,2));
    }
    for (const width of [1440,390]) {
      await cmd('Emulation.setDeviceMetricsOverride',{width,height:width===1440?1050:844,deviceScaleFactor:1,mobile:false});
      const label=`theme-${mode}-${width}`;
      for (const [hash,name] of [['#library','library'],['#meeting/paper','workspace'],['#queue','queue'],['#settings','settings'],['#diagnostics','diagnostics'],['#help','help']]) {
        await nav(hash);
        // A same-hash visit after resizing can retain the previous mobile scroll offset.
        await evaluate('window.scrollTo(0,0)');
        check(await evaluate('document.documentElement.scrollWidth<=innerWidth'),`${label}/${name}: no page overflow`);
        check(await evaluate('(()=>{const r=document.querySelector("#theme-preference").getBoundingClientRect();return r.width>0&&r.left>=0&&r.right<=innerWidth&&r.top>=0;})()'),`${label}/${name}: theme selector visible`);
        await sampleContrast(`${label}/${name}`,['h1','.muted','.status.completed','.status.running','.status.failed','.status.queued','.tag','.notice','.notice.warn','.notice.info','.audio-top','.audio-controls time','button.primary','.theme-picker','input:not([type=checkbox]):not([type=range])','select','label','.topic p','.action-item small','.page-footer']);
        await sampleContrast(`${label}/${name}/control`,['input:not([type=checkbox]):not([type=range])','select','button.primary'],3,'borderTopColor');
        if (mode === 'dark') await sampleContrast(`${label}/${name}/accent`,['.sidebar nav a.active .nav-icon','.timestamp','.tabs button.active','.settings-nav button.active','.audio-controls .play','.back']);
        if (hash==='#library') {
          const buttonRect=await evaluate('(()=>{const r=document.querySelector("button.primary").getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()');
          await cmd('Input.dispatchMouseEvent',{type:'mouseMoved',...buttonRect}); await new Promise(r=>setTimeout(r,170));
          await sampleContrast(`${label}/hover`,['button.primary:hover']);
        }
        if (hash==='#settings') {
          for (const group of ['本地转写','Pi 纪要','存储','通知','录音与检测']) {
            await click(`[data-settings-group="${group}"]`);
            await sampleContrast(`${label}/settings/${group}`,['label','.field-help','input:not([type=checkbox])','select','.notice']);
          }
        }
        await cleanSnap(`${label}-${name}`);
      }
      await nav('#meeting/paper');
      for (const [action,name] of [['edit','edit-dialog'],['retranscribe','reprocess-dialog'],['delete','delete-dialog']]) {
        await click(`[data-action=${action}]`);
        check(await evaluate('document.querySelector("#modal").open && document.querySelector("#modal").getBoundingClientRect().right<=innerWidth'),`${label}/${name}: bounded modal`);
        await sampleContrast(`${label}/${name}`,['#modal h2','#modal p','#modal label','#modal input','#modal button.danger','#modal button.primary']);
        await evaluate('document.querySelector("#modal [data-action=close-modal]").focus()'); await key('Tab');
        const focus=await evaluate('({visible:!!document.querySelector("#modal :focus-visible"),width:getComputedStyle(document.activeElement).outlineWidth,element:document.activeElement.outerHTML})');
        check(focus.visible && parseFloat(focus.width)>=3,`${label}/${name}: keyboard focus outline visible: ${JSON.stringify(focus)}`);
        await sampleContrast(`${label}/${name}/focus`,['#modal :focus-visible'],3,'outlineColor');
        await cleanSnap(`${label}-${name}`); await click('#modal [data-action=close-modal]');
      }
      await nav('#meeting/island'); await sampleContrast(`${label}/failed`,['.notice.warn','.status.failed']);
      await cleanSnap(`${label}-failed-workspace`);
      await nav('#meeting/blank'); await sampleContrast(`${label}/empty`,['.empty-state p','.empty-state h2']);
      await cleanSnap(`${label}-empty-workspace`);
      await nav('#library'); await input('#meeting-query','不存在的虚构词');
      await sampleContrast(`${label}/no-results`,['.empty-state p','.empty-state h2']);
      await cleanSnap(`${label}-no-results`); await click('[data-action=clear-filters]');
    }
  }
  // Sidebar is isolated presentation state: no business remounts or real operations.
  const sidebarMode = () => evaluate('document.documentElement.dataset.sidebar');
  const setSidebar = async mode => { if (await sidebarMode() !== mode) await click('#sidebar-toggle'); };
  const iconGeometry = await evaluate('[...document.querySelectorAll(".sidebar .nav-icon")].map(svg=>{const b=svg.getBBox();return {name:svg.dataset.icon,viewBox:svg.getAttribute("viewBox"),stroke:getComputedStyle(svg).strokeWidth,width:svg.getBoundingClientRect().width,height:svg.getBoundingClientRect().height,bounds:{x:b.x,y:b.y,width:b.width,height:b.height},length:[...svg.children].reduce((sum,node)=>sum+(node.getTotalLength?.()||0),0),decorative:svg.getAttribute("aria-hidden"),focusable:svg.getAttribute("focusable")};})');
  for (const icon of iconGeometry) {
    check(icon.viewBox==='0 0 24 24' && icon.stroke==='1.8px' && icon.width===20 && icon.height===20 && icon.decorative==='true' && icon.focusable==='false', `${icon.name}: consistent local decorative SVG stroke/canvas`);
    check(icon.bounds.width>=16 && icon.bounds.height>=16 && icon.bounds.x>=2 && icon.bounds.y>=2 && icon.bounds.x+icon.bounds.width<=22 && icon.bounds.y+icon.bounds.height<=22, `${icon.name}: visible shape uses comparable unclipped bounds`);
  }
  check(Math.max(...iconGeometry.map(x=>x.length))/Math.min(...iconGeometry.map(x=>x.length))<1.8, 'icon stroke-length proxy avoids overly sparse or heavy symbols');
  await writeFile(path.join(output,'sidebar-icon-geometry.json'),JSON.stringify(iconGeometry,null,2));
  const sidebarStyles = [];
  const logoSamples = [];
  for (const mode of ['light','dark']) {
    await chooseTheme(mode); await nav('#meeting/paper');
    for (const width of [1440,390]) {
      await cmd('Emulation.setDeviceMetricsOverride',{width,height:width===1440?1050:844,deviceScaleFactor:1,mobile:false});
      for (const layout of ['expanded','collapsed']) {
        await setSidebar(layout);
        const label=`sidebar-${mode}-${layout}-${width}`;
        check(await evaluate(`document.querySelector('#sidebar-toggle').getAttribute('aria-expanded')==='${layout==='expanded'}'`), `${label}: toggle expansion semantics`);
        check(await evaluate('document.documentElement.scrollWidth<=innerWidth && document.querySelectorAll("[data-nav][aria-current=page]").length===1 && document.querySelector("[data-nav=library]").getAttribute("aria-current")==="page"'), `${label}: no overflow and correct current location`);
        check(await evaluate('[...document.querySelectorAll("[data-sidebar-tip]")].every(e=>e.getAttribute("aria-label")?.length>1 && e.getBoundingClientRect().width>0)'), `${label}: every rail entry, status and reset has a name`);
        const style=await evaluate('(()=>{const root=getComputedStyle(document.documentElement),icon=getComputedStyle(document.querySelector("[data-nav=library] svg")),active=getComputedStyle(document.querySelector("[data-nav=library]"),"::before");return {background:root.backgroundColor,ink:root.color,icon:icon.color,indicator:active.content,indicatorWidth:active.width};})()');
        sidebarStyles.push({label,...style});
        const logo = await evaluate('(()=>{const tile=document.querySelector(".brand-mark"),svg=tile.querySelector("svg"),r=tile.getBoundingClientRect(),g=svg.getBoundingClientRect(),b=svg.getBBox(),s=getComputedStyle(tile);return {name:document.querySelector(".brand").getAttribute("aria-label"),letters:[...svg.querySelectorAll("path")].map(p=>p.dataset.letter).join(""),strokes:[...svg.querySelectorAll("path")].map(p=>getComputedStyle(p).stroke),hidden:svg.getAttribute("aria-hidden"),focusable:svg.getAttribute("focusable"),tile:{width:r.width,height:r.height,radius:s.borderRadius,background:s.backgroundColor,color:s.color},glyph:{width:g.width,height:g.height,stroke:getComputedStyle(svg).stroke,x:b.x,y:b.y,right:b.x+b.width,bottom:b.y+b.height},inside:g.left>=r.left&&g.right<=r.right&&g.top>=r.top&&g.bottom<=r.bottom};})()');
        logo.letterContrast = logo.strokes.map(color=>contrast(color,logo.tile.background));
        logoSamples.push({label,...logo});
        const expectedO = mode==='light'?'rgb(0, 3, 4)':'rgb(33, 59, 120)';
        check(logo.strokes[0]===expectedO && logo.strokes[1]===logo.tile.color && logo.strokes[2]===logo.tile.color && new Set(logo.strokes).size===2, `${label}: o has a distinct local ink color while both m letters retain the original color`);
        check(logo.letterContrast.every(ratio=>ratio>=4.5), `${label}: every letter stays readable on its existing brand background`);
        check(logo.letters==='omm' && logo.name==='Open Meeting Memo 会议库' && logo.hidden==='true' && logo.focusable==='false', `${label}: self-drawn omm keeps accessible brand name without extra focus stops`);
        const logoSize=width===390?35:layout==='collapsed'?38:43;
        check(logo.tile.width===logoSize && logo.tile.height===logoSize && logo.tile.radius==='12px' && logo.glyph.stroke===logo.tile.color, `${label}: existing rounded brand tile size and inherited colors preserved`);
        check(logo.inside && Math.abs(logo.glyph.width/logo.glyph.height-46/24)<.005 && logo.glyph.x>=2 && logo.glyph.y>=2 && logo.glyph.right<=44 && logo.glyph.bottom<=22, `${label}: wordmark remains centered, unclipped and undistorted at actual size`);
        check(style.indicator!=='none' && parseFloat(style.indicatorWidth)>0, `${label}: active location also has a visible shape marker`);
        await sampleContrast(label,['.sidebar nav a.active .nav-icon','.sidebar-status','.sidebar-reset','.sidebar-toggle']);
        if (layout==='collapsed') {
          check(await evaluate('[...document.querySelectorAll(".nav-count:not(:empty)")].every(count=>{const c=count.getBoundingClientRect(),svg=count.parentElement.querySelector("svg"),r=svg.getBoundingClientRect(),b=svg.getBBox(),scale=r.width/24;return c.width>0 && c.left>=r.left+(b.x+b.width+.9)*scale && c.right<=innerWidth;})'), `${label}: counts remain visible without covering icon strokes`);
          const hover=await evaluate('(()=>{const r=document.querySelector("[data-nav=library]").getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()');
          await cmd('Input.dispatchMouseEvent',{type:'mouseMoved',...hover}); await wait('!document.querySelector("#sidebar-tooltip").hidden');
          check(await evaluate('document.querySelector("#sidebar-tooltip").textContent.includes("会议库") && document.querySelector("#sidebar-tooltip").textContent.includes(document.querySelector("#library-count").textContent)'), `${label}: hover reveals name and meeting count`);
          await evaluate('document.querySelector("[data-nav=library]").focus()'); await key('Tab');
          check(await evaluate('document.activeElement.matches("[data-nav=queue]:focus-visible") && document.activeElement.getAttribute("aria-describedby")==="sidebar-tooltip" && document.querySelector("#sidebar-tooltip").textContent.includes("处理队列")'), `${label}: keyboard reveals queue name and visible focus`);
          check(await evaluate('(()=>{const r=document.querySelector("#sidebar-tooltip").getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.top>=0&&r.bottom<=innerHeight;})()'), `${label}: tooltip stays in viewport`);
          await sampleContrast(label+'/focus',['.sidebar nav :focus-visible'],3,'outlineColor');
          await key('Escape'); check(await evaluate('document.querySelector("#sidebar-tooltip").hidden'), `${label}: Escape dismisses hint`);
        }
        await evaluate('document.activeElement?.blur();document.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true}));');
        await cleanSnap(label);
      }
    }
  }
  // Icon-only navigation still uses the existing router and confirmation flows.
  await cmd('Emulation.setDeviceMetricsOverride',{width:1440,height:1050,deviceScaleFactor:1,mobile:false});
  for (const layout of ['collapsed','expanded']) {
    await setSidebar(layout);
    for (const page of ['library','queue','settings','diagnostics','help']) {
      await click(`[data-nav=${page}]`); await wait(`location.hash==='#${page}'`);
      check(await evaluate(`document.querySelector('[data-nav=${page}]').getAttribute('aria-current')==='page' && document.querySelector('#main h1')`), `${layout}: ${page} navigation works`);
    }
  }
  await nav('#library'); await input('#meeting-query','纸舟');
  await evaluate('window.sidebarBusinessNode=document.querySelector("#main").firstElementChild');
  await evaluate('window.sidebarBusinessSnapshot=JSON.stringify(DemoTest.getState())');
  await setSidebar('collapsed'); await chooseTheme('light');
  check(await sidebarMode()==='collapsed' && await evaluate('document.querySelector("#meeting-query").value==="纸舟" && document.querySelector("#main").firstElementChild===window.sidebarBusinessNode'), 'theme and rail preserve filtered content without remount');
  await setSidebar('expanded'); check(await effectiveTheme()==='light', 'sidebar mode never changes chosen theme');
  await nav('#meeting/paper'); await input('#audio-progress',123); await click('[data-action=edit]'); await input('#edit-title','侧栏切换不丢失的草稿');
  await setSidebar('collapsed');
  check(await evaluate('document.querySelector("#modal").open && document.querySelector("#edit-title").value==="侧栏切换不丢失的草稿" && Number(document.querySelector("#audio-progress").value)===123'), 'rail keeps modal draft, current meeting and silent playback position');
  await click('#modal [data-action=close-modal]');
  await nav('#settings'); await input('#setting-segment_ms',60000); await setSidebar('expanded');
  check(await evaluate('document.querySelector("#setting-segment_ms").value==="60000" && document.querySelector("#save-state").textContent.includes("未保存") && JSON.stringify(DemoTest.getState())===window.sidebarBusinessSnapshot'), 'rail preserves settings draft and business data');
  await click('[data-action=discard-settings]'); await confirm();
  await setSidebar('collapsed'); await reset(); check(await sidebarMode()==='collapsed', 'demo reset does not overwrite sidebar preference');
  await reloadDemo(); check(await sidebarMode()==='collapsed' && await evaluate('document.querySelector("#sidebar-toggle").getAttribute("aria-expanded")==="false"'), 'saved collapsed layout is restored on refresh');
  await evaluate('localStorage.setItem("omm-webui-sidebar-v1","broken")'); await reloadDemo();
  check(await sidebarMode()==='expanded' && await evaluate('!document.querySelector("#sidebar-warning").hidden'), 'corrupt sidebar preference degrades to expanded with explanation');
  await setSidebar('collapsed'); check(await evaluate('document.querySelector("#sidebar-warning").hidden && localStorage.getItem("omm-webui-sidebar-v1")==="collapsed"'), 'valid toggle repairs corrupt sidebar preference');
  await setSidebar('expanded');
  const sidebarDeniedWrite = await cmd('Page.addScriptToEvaluateOnNewDocument',{source:'const sidebarSet=Storage.prototype.setItem;Storage.prototype.setItem=function(k,v){if(k==="omm-webui-sidebar-v1")throw Error("quota");return sidebarSet.call(this,k,v)};'});
  await reloadDemo(); await setSidebar('collapsed');
  check(await sidebarMode()==='collapsed' && await evaluate('document.querySelector("#sidebar-warning").textContent.includes("此前保存") && localStorage.getItem("omm-webui-sidebar-v1")==="expanded"'), 'denied sidebar writes keep in-memory layout and accurately describe old preference');
  await cmd('Page.removeScriptToEvaluateOnNewDocument',{identifier:sidebarDeniedWrite.identifier}); await reloadDemo();
  check(await sidebarMode()==='expanded', 'refresh after denied write restores previously saved expanded layout');
  const sidebarDenied = await cmd('Page.addScriptToEvaluateOnNewDocument',{source:'const sidebarGet=Storage.prototype.getItem,sidebarSet=Storage.prototype.setItem;Storage.prototype.getItem=function(k){if(k==="omm-webui-sidebar-v1")throw Error("blocked");return sidebarGet.call(this,k)};Storage.prototype.setItem=function(k,v){if(k==="omm-webui-sidebar-v1")throw Error("quota");return sidebarSet.call(this,k,v)};'});
  await reloadDemo(); check(await sidebarMode()==='expanded' && await evaluate('!document.querySelector("#sidebar-warning").hidden'), 'sidebar read denial defaults expanded without breaking page');
  await setSidebar('collapsed'); await chooseTheme('dark'); check(await sidebarMode()==='collapsed' && await effectiveTheme()==='dark', 'denied storage still supports independent sidebar and theme controls');
  await cmd('Page.removeScriptToEvaluateOnNewDocument',{identifier:sidebarDenied.identifier}); await reloadDemo(); await setSidebar('expanded');
  await writeFile(path.join(output,'sidebar-styles.json'),JSON.stringify(sidebarStyles,null,2));
  await writeFile(path.join(output,'logo-samples.json'),JSON.stringify(logoSamples,null,2));
  await writeFile(path.join(output,'theme-contrast.json'),JSON.stringify(contrastSamples,null,2));
  await chooseTheme('system'); await emulateTheme('light'); await reset();
  // Storage failures and malformed data must degrade to working, resettable UI.
  await evaluate('localStorage.setItem("omm-webui-fictional-v1","{invalid")'); await cmd('Page.reload'); await new Promise(r => setTimeout(r, 200)); await wait('!!window.DemoTest');
  check(!(await evaluate('document.querySelector("#storage-warning").hidden')), 'corrupt storage degrades visibly');
  check((await evaluate('DemoTest.getState().meetings.length')) === 6, 'corrupt storage resets fixtures');
  await cmd('Page.addScriptToEvaluateOnNewDocument', { source: 'Storage.prototype.getItem=function(){throw Error("blocked")};Storage.prototype.setItem=function(){throw Error("quota")};' });
  await cmd('Page.reload'); await new Promise(r => setTimeout(r, 200)); await wait('!!window.DemoTest'); await reset();
  check(!(await evaluate('document.querySelector("#storage-warning").hidden')), 'read/write denied storage remains usable');
  check(await evaluate('document.documentElement.dataset.themePreference==="system" && !document.querySelector("#theme-warning").hidden'), 'theme read denied follows OS with visible warning');
  await chooseTheme('dark'); check(await effectiveTheme()==='dark', 'theme fully denied storage still switches in memory');
  await emulateTheme('light'); check(await effectiveTheme()==='dark', 'in-memory manual theme is not overwritten by OS');
  await chooseTheme('system'); check(await effectiveTheme()==='light', 'denied storage can return to system mode');
  // Chrome date inputs load their built-in calendar icon as a data: SVG.
  check(requests.every(url => /^(file:|data:)/.test(url)), `only offline file/data resources: ${requests.join(', ')}`);
  check(browserErrors.length === 0, `no browser errors: ${browserErrors.join('\n')}`);
  await writeFile(path.join(output, 'results.json'), JSON.stringify({ passed: true, checks, requests: [...new Set(requests)], browserErrors }, null, 2));
  console.log(`PASS ${checks} assertions; no external requests or browser errors. Evidence: ${output}`);
} catch (error) {
  failures.push(error.stack); console.error(error.stack); process.exitCode = 1;
  await writeFile(path.join(output, 'failure.txt'), failures.join('\n')).catch(() => {});
} finally {
  if (ws?.readyState === WebSocket.OPEN) ws.close();
  if (child && child.exitCode === null) { const exited = once(child, 'exit'); child.kill('SIGTERM'); await exited; }
  await rm(profile, { recursive: true, force: true });
}
