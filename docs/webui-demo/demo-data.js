/* Fictional fixtures only. No personal meeting, installation or credential data. */
(function (root) {
  'use strict';
  const transcript = [
    [0, 'system', '今天我们聚焦「纸舟」阅读计划的第一版体验：让读者更容易找到一本想读的书，再把阅读安排进生活。'],
    [46, 'mic', '从虚构的体验走查来看，首页信息有点多。我们建议保留最近在读和每周书单，把排行榜放到探索页。'],
    [134, 'system', '同意。先把核心路径缩短：选书、加入书架、设定一个小目标。第一版不做社交动态。'],
    [267, 'mic', '关于阅读提醒，我们采用用户主动开启的方式，默认不发通知。提醒文案保持克制，避免制造阅读负担。'],
    [438, 'system', '书单详情需要解释为什么推荐这本书。编辑推荐理由控制在两句话，保留主题标签和预计阅读时长。'],
    [631, 'mic', '我来整理书单卡片与空书架的设计，下周三给出两个方案。走查时也覆盖窄屏和大字体。'],
    [865, 'system', '检索先支持书名和作者。全库语义搜索留到后续，不放进这一轮交付范围。'],
    [1096, 'mic', '评估方式就看从书单到加入书架的路径能否顺利完成，邀请几位自愿体验者，用虚构账户进行走查。'],
    [1402, 'system', '还有一个风险：书籍封面授权需要先确认。第一版用已授权素材，未确认的不要上线。'],
    [1664, 'mic', '最后确认：本周完成信息架构，下周三评审卡片和空状态；素材清单由编辑组整理。我们会在下一次评审复核这三件事。']
  ].map(([at, source, text]) => ({ at, source, text }));
  const summary = {
    intro: '本次讨论确定了「纸舟」阅读计划第一版的信息架构与体验边界。优先打通选书到加入书架的核心路径，保留温和、主动选择的阅读提醒。',
    decisions: ['首页聚焦「最近在读」与「每周书单」，排行榜移到探索页。', '第一版支持书名、作者检索；暂不加入社交动态与全库语义搜索。', '提醒默认关闭，由读者主动开启；封面仅使用已授权素材。'],
    topics: [
      { title: '01 / 让选书更轻松', at: 46, text: '减少首页并列入口。每张书单卡片给出两句话的推荐理由，并展示主题与预计阅读时长。' },
      { title: '02 / 用小目标代替阅读压力', at: 267, text: '阅读提醒由用户主动选择，不用连续打卡推动留存。空书架应该帮助用户开始，而不是强调空白。' },
      { title: '03 / 先验证核心路径', at: 1096, text: '邀请自愿体验者使用虚构账户走查，从书单浏览到加入书架；同时覆盖窄屏和大字体。' }
    ],
    actions: [
      { text: '整理首页信息架构，明确入口优先级', owner: '林禾（虚构）', due: '10 月 19 日' },
      { text: '输出书单卡片与空书架两套设计', owner: '许青（虚构）', due: '10 月 21 日' },
      { text: '核对封面素材授权，形成可用清单', owner: '周言（虚构）', due: '10 月 20 日' }
    ]
  };
  const meetings = [
    { id: 'paper', title: '纸舟阅读计划 · 第一版体验评审', date: '2026-10-16', time: '14:00', duration: 1860, source: '飞书', status: 'completed', note: '聚焦选书、书架与阅读提醒的核心路径。', tags: ['产品设计', '纸舟'], transcript, summary, audio: true, segments: 2, completedSegments: 2, versions: [{ name: '初次处理', status: 'completed', stage: 'all' }], activeVersion: 0 },
    { id: 'garden', title: '口袋花园 · 每周进展同步', date: '2026-10-17', time: '10:30', duration: 2540, source: '飞书', status: 'running', note: '虚构项目：室内植物养护手册。', tags: ['每周同步'], transcript: transcript.slice(0, 4), summary: null, audio: true, segments: 2, completedSegments: 1, versions: [{ name: '初次处理', status: 'running', stage: 'all' }], activeVersion: 0 },
    { id: 'island', title: '小岛书店 · 秋日活动策划', date: '2026-10-15', time: '15:00', duration: 1680, source: '手动录制', status: 'failed', error: 'Pi 纪要未完成：专用认证不可用（演示故障）。逐字稿已保存，请先确认认证，再只重试纪要。', note: '用来体验纪要失败与恢复。', tags: ['活动'], transcript: transcript.slice(0, 7), summary: null, audio: true, segments: 1, completedSegments: 1, versions: [{ name: '初次处理', status: 'failed', stage: 'summary' }], activeVersion: 0 },
    { id: 'trail', title: '山径地图 · 路线说明走查', date: '2026-10-14', time: '09:30', duration: 1860, source: '飞书', status: 'completed', note: '演示样本，非真实路线或安全建议。', tags: ['内容走查'], transcript, summary, audio: true, segments: 1, completedSegments: 1, versions: [{ name: '初次处理', status: 'completed', stage: 'all' }], activeVersion: 0 },
    { id: 'blank', title: '随手记 · 空内容示例', date: '2026-10-13', time: '17:10', duration: 0, source: '手动录制', status: 'empty', note: '没有有效音频帧，不创建处理任务。', tags: [], transcript: [], summary: null, audio: false, segments: 0, completedSegments: 0, versions: [], activeVersion: -1 },
    { id: 'radio', title: '周末电台 · 节目选题会', date: '2026-10-12', time: '11:00', duration: 2220, source: '导入样例', status: 'queued', note: '导入是 WebUI 新能力提案；这里只生成样例，不读取文件。', tags: ['灵感'], transcript: [], summary: null, audio: true, segments: 2, completedSegments: 0, versions: [{ name: '初次处理', status: 'queued', stage: 'all' }], activeVersion: 0 }
  ];
  const fields = [
    { group: '录音与检测', key: 'audio.opus_bitrate_kbps', label: '归档码率', value: 48, options: [32, 48, 64, 96], unit: 'kbps', hint: '新片段入队时冻结；不会重编码已有录音。' },
    { group: '录音与检测', key: 'segment_ms', label: '录音分段时长', value: 1800000, type: 'number', min: 60000, max: 1800000, unit: '毫秒', hint: '1–30 分钟；按帧切分，会议仍是一份完整记录。' },
    { group: '录音与检测', key: 'start_delay_ms', label: '强信号确认等待', value: 15000, type: 'number', min: 0, unit: '毫秒', hint: '窗口或进程信号不能单独触发录制，等待可能漏掉开头。' },
    { group: '录音与检测', key: 'end_grace_ms', label: '会议结束宽限', value: 30000, type: 'number', min: 0, unit: '毫秒', hint: '观察失败是未知，不直接当作会议结束。' },
    { group: '录音与检测', key: 'poll_ms', label: '检测轮询间隔', value: 2000, type: 'number', min: 100, max: 60000, unit: '毫秒' },
    { group: '录音与检测', key: 'capture_executable', label: '录音程序路径', value: '/demo/omm/OMMCapture.app/Contents/MacOS/omm-capture', path: true },
    { group: '录音与检测', key: 'lark_executable', label: '飞书 CLI 路径（可选）', value: '/demo/bin/lark-cli', path: true, optional: true },
    { group: '录音与检测', key: 'rtc_log_dirs', label: 'RTC 日志目录', value: '/demo/lark/rtc', type: 'textarea', hint: '每行一个绝对路径；这是虚构路径，不扫描任何目录。', pathList: true },
    { group: '本地转写', key: 'asr.threads', label: '请求转写线程数', value: 3, type: 'number', min: 1, unit: '线程', hint: '真实执行按机器预算限制为 1–3 线程，单 worker；非硬性 CPU 配额。' },
    { group: '本地转写', key: 'asr.timeout_ms', label: '转写超时', value: 7200000, type: 'number', min: 1, unit: '毫秒' },
    { group: '本地转写', key: 'worker_executable', label: '转写 worker 路径', value: '/demo/omm/bin/omm-asr-worker', path: true },
    ...[['library_path', '原生 C API 库', '/demo/models/libsherpa-onnx-c-api.dylib'], ['encoder_path', '编码模型', '/demo/models/encoder.int8.onnx'], ['decoder_path', '解码模型', '/demo/models/decoder.int8.onnx'], ['tokens_path', '词表', '/demo/models/tokens.txt'], ['vad_path', '语音活动检测模型', '/demo/models/silero_vad.onnx']].map(([key, label, value]) => ({ group: '本地转写', key: 'asr.' + key, label, value, path: true })),
    { group: 'Pi 纪要', key: 'pi.provider', label: '模型提供方', value: 'demo-provider', hint: '填写专用 Pi 已配置的提供方名称，不自动切换。' },
    { group: 'Pi 纪要', key: 'pi.model', label: '模型名称', value: 'demo-text-model', hint: '虚构名称；不会查询模型或发起调用。' },
    { group: 'Pi 纪要', key: 'pi.executable', label: 'Pi 稳定入口', value: '/demo/bin/pi', path: true },
    { group: 'Pi 纪要', key: 'pi.node_executable', label: 'Node 运行时（可选）', value: '/demo/bin/node', path: true, optional: true },
    { group: 'Pi 纪要', key: 'pi.prompt_version', label: '提示版本', value: 'minutes-v1' },
    { group: 'Pi 纪要', key: 'pi.timeout_ms', label: '单次纪要超时', value: 300000, type: 'number', min: 1, unit: '毫秒' },
    { group: 'Pi 纪要', key: 'pi.max_input_bytes', label: '输入上限', value: 131072, type: 'number', min: 1024, unit: '字节' },
    { group: 'Pi 纪要', key: 'pi.max_output_bytes', label: '输出上限', value: 131072, type: 'number', min: 1, unit: '字节' },
    { group: '通知', key: 'notifications.enabled', label: '启用桌面状态通知', value: true, type: 'checkbox', hint: '此处仅模拟开关，绝不发送桌面通知。' },
    { group: '通知', key: 'notifications.show_meeting_title', label: '通知中显示会议标题', value: true, type: 'checkbox', hint: '不发送正文；隐藏标题也需遵循系统锁屏设置。' }
  ];
  const events = [['recording_started', '录音开始'], ['recording_stopped', '录音封存'], ['summary_ready', '纪要完成'], ['recording_failed', '录音失败'], ['asr_failed', '转写失败'], ['summary_failed', '纪要失败']];
  // Calendar dates use local components, never UTC serialization of a local instant.
  function localDay(date = new Date()) {
    return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
  }
  function parseDay(value) {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
    const [year, month, day] = value.split('-').map(Number);
    const date = new Date(year, month - 1, day, 12);
    return localDay(date) === value ? date : null;
  }
  function shiftDay(value, count) {
    const date = parseDay(value);
    if (!date) throw new Error('invalid fictional date');
    date.setDate(date.getDate() + count);
    return localDay(date);
  }
  function dateRange(preset, now = new Date()) {
    const to = localDay(now);
    const offset = preset === 'today' ? 0 : preset === 'week' ? (parseDay(to).getDay() + 6) % 7 : preset === 'three' ? 2 : 6;
    return { from: shiftDay(to, -offset), to };
  }
  function fresh(now = new Date()) {
    const anchorDate = localDay(now);
    const samples = meetings.map((m, i) => ({ ...m, date: shiftDay(anchorDate, -[1, 0, 2, 3, 4, 5][i]) }));
    return JSON.parse(JSON.stringify({ schema: 2, anchorDate, meetings: samples, config: Object.fromEntries(fields.map(f => [f.key, f.value])), events: events.map(e => e[0]), auto: false, consent: false, recording: null }));
  }
  function migrate(value, now = new Date()) {
    if (!value || ![1, 2].includes(value.schema)) return value;
    if (value.schema === 1) {
      // Count plain calendar days with UTC arithmetic, not a local instant-to-UTC conversion.
      const ordinal = day => { const d = parseDay(day); return d ? Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 86400000 : NaN; };
      const anchorDate = localDay(now);
      if (Array.isArray(value.meetings)) for (const m of value.meetings) {
        if (m && parseDay(m.date)) m.date = shiftDay(anchorDate, ordinal(m.date) - ordinal('2026-10-17'));
      }
      value.schema = 2; value.anchorDate = anchorDate;
    }
    if (Array.isArray(value.meetings)) for (const m of value.meetings) {
      for (const artifact of [m, ...(Array.isArray(m?.versions) ? m.versions.map(v => v?.snapshot) : [])]) {
        if (Array.isArray(artifact?.summary?.actions)) for (const action of artifact.summary.actions) {
          if (action && typeof action === 'object') delete action.done;
        }
      }
    }
    return value;
  }
  root.DemoData = { fresh, migrate, localDay, parseDay, shiftDay, dateRange, fields, events, sampleTranscript: transcript, sampleSummary: summary };
})(typeof window === 'undefined' ? globalThis : window);
