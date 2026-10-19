/* Runs before styles/first paint. Only a non-sensitive appearance preference is stored. */
(function () {
  'use strict';
  const key = 'omm-webui-theme-v1';
  const modes = ['system', 'light', 'dark'];
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  let preference = 'system';
  let warning = '';
  try {
    const saved = localStorage.getItem(key);
    if (saved !== null && !modes.includes(saved)) warning = '外观偏好已损坏，已恢复跟随系统。重新选择可保存偏好。';
    else if (saved) preference = saved;
  } catch { warning = '浏览器无法读取外观偏好；本次仍可切换，刷新后将跟随系统。'; }
  function apply() {
    document.documentElement.dataset.theme = preference === 'system' ? (media.matches ? 'dark' : 'light') : preference;
    document.documentElement.dataset.themePreference = preference;
    const picker = document.querySelector('#theme-preference');
    if (picker) picker.value = preference;
    const status = document.querySelector('#theme-warning');
    if (status) { status.textContent = warning; status.hidden = !warning; }
  }
  function choose(value) {
    if (!modes.includes(value)) return;
    preference = value;
    try { localStorage.setItem(key, preference); warning = ''; }
    catch { warning = '浏览器无法保存外观偏好；本次仍可切换，刷新后可能恢复此前保存的外观或跟随系统。'; }
    apply();
  }
  apply();
  media.addEventListener('change', () => { if (preference === 'system') apply(); });
  document.addEventListener('DOMContentLoaded', () => {
    apply();
    document.querySelector('#theme-preference').addEventListener('change', event => choose(event.target.value));
  });
})();
