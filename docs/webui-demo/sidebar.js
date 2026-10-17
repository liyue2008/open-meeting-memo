/* Local sidebar preference only. Self-drawn SVGs are static markup in index.html. */
(function () {
  'use strict';
  const key = 'omm-webui-sidebar-v1';
  let mode = 'expanded';
  let warning = '';
  let tipTarget = null;
  let tipTimer;
  try {
    const saved = localStorage.getItem(key);
    if (saved === 'collapsed' || saved === 'expanded') mode = saved;
    else if (saved !== null) warning = '侧栏偏好已损坏，已默认展开；重新切换可保存偏好。';
  } catch { warning = '无法读取侧栏偏好，已默认展开；本次仍可收缩或展开。'; }
  document.documentElement.dataset.sidebar = mode;
  document.addEventListener('DOMContentLoaded', () => {
    const toggle = document.querySelector('#sidebar-toggle');
    const tip = document.querySelector('#sidebar-tooltip');
    const notice = document.querySelector('#sidebar-warning');
    const targets = [...document.querySelectorAll('[data-sidebar-tip]')];
    const label = target => {
      const count = target.querySelector('.nav-count')?.textContent;
      return target.dataset.sidebarTip + (count ? ` · ${count}${target.dataset.nav === 'library' ? ' 场虚构会议' : ' 项需关注处理'}` : '');
    };
    function hideTip() {
      clearTimeout(tipTimer);
      tipTarget?.removeAttribute('aria-describedby');
      tipTarget = null; tip.hidden = true;
    }
    function showTip(target) {
      clearTimeout(tipTimer);
      if (mode !== 'collapsed' && target !== toggle) return;
      tipTarget?.removeAttribute('aria-describedby');
      tipTarget = target;
      tip.textContent = label(target); tip.hidden = false;
      target.setAttribute('aria-describedby', tip.id);
      const box = target.getBoundingClientRect();
      const mobile = window.matchMedia('(max-width: 650px)').matches;
      const left = mobile ? box.x + box.width / 2 - tip.offsetWidth / 2 : box.right + 12;
      const top = mobile ? box.bottom + 8 : box.y + box.height / 2 - tip.offsetHeight / 2;
      tip.style.left = `${Math.max(8, Math.min(left, innerWidth - tip.offsetWidth - 8))}px`;
      tip.style.top = `${Math.max(8, Math.min(top, innerHeight - tip.offsetHeight - 8))}px`;
    }
    function leaveTip(target) {
      if (!target.matches(':focus-visible')) tipTimer = setTimeout(hideTip, 140);
    }
    function updateNames() {
      for (const target of targets) target.setAttribute('aria-label', label(target));
      if (tipTarget) showTip(tipTarget);
    }
    function apply() {
      document.documentElement.dataset.sidebar = mode;
      const expanded = mode === 'expanded';
      toggle.setAttribute('aria-expanded', String(expanded));
      toggle.dataset.sidebarTip = expanded ? '收起侧栏' : '展开侧栏';
      toggle.querySelector('.sidebar-toggle-label').textContent = toggle.dataset.sidebarTip;
      notice.textContent = warning; notice.hidden = !warning;
      hideTip(); updateNames();
    }
    toggle.addEventListener('click', () => {
      mode = mode === 'expanded' ? 'collapsed' : 'expanded';
      try { localStorage.setItem(key, mode); warning = ''; }
      catch { warning = '无法保存侧栏偏好；本次仍可切换，刷新后可能恢复此前保存的布局或默认展开。'; }
      apply();
      if (toggle.matches(':focus-visible')) showTip(toggle);
    });
    for (const target of targets) {
      target.addEventListener('mouseenter', () => showTip(target));
      target.addEventListener('mouseleave', () => leaveTip(target));
      target.addEventListener('focus', () => showTip(target));
      target.addEventListener('blur', hideTip);
      target.addEventListener('click', () => { if (target !== toggle) hideTip(); });
    }
    tip.addEventListener('mouseenter', () => clearTimeout(tipTimer));
    tip.addEventListener('mouseleave', () => { if (!tipTarget?.matches(':focus-visible')) hideTip(); });
    document.addEventListener('keydown', event => { if (event.key === 'Escape') hideTip(); });
    window.addEventListener('resize', hideTip);
    window.addEventListener('scroll', hideTip, { passive: true });
    // Counts belong to the business UI; observe only their text to keep rail names current.
    const counts = new MutationObserver(updateNames);
    for (const node of document.querySelectorAll('.nav-count')) counts.observe(node, { childList: true, subtree: true });
    apply();
  });
})();
