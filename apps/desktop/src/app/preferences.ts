import { useEffect, useState } from 'react';
export type Theme = 'system' | 'light' | 'dark';
export function useAppearance() {
  const [warning, setWarning] = useState('');
  const [theme, setTheme] = useState<Theme>(() => {
    try {
      const value = localStorage.getItem('omm.theme');
      if (value === null || value === 'system') return 'system';
      if (value === 'dark' || value === 'light') return value;
    } catch {
      /* reported below */
    }
    return 'system';
  });
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem('omm.sidebar') === 'collapsed';
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      const t = localStorage.getItem('omm.theme');
      const s = localStorage.getItem('omm.sidebar');
      if (
        (t !== null && !['system', 'dark', 'light'].includes(t)) ||
        (s !== null && !['collapsed', 'expanded'].includes(s))
      )
        setWarning('外观偏好损坏，已使用默认布局。');
    } catch {
      setWarning('无法读取外观偏好；本次更改只在当前窗口生效。');
    }
  }, []);
  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {
      document.documentElement.dataset.theme =
        theme === 'system' ? (media.matches ? 'dark' : 'light') : theme;
    };
    apply();
    media.addEventListener('change', apply);
    return () => media.removeEventListener('change', apply);
  }, [theme]);
  useEffect(() => {
    document.documentElement.dataset.sidebar = collapsed ? 'collapsed' : 'expanded';
  }, [collapsed]);
  const save = (key: string, value: string) => {
    try {
      localStorage.setItem(key, value);
    } catch {
      setWarning('无法保存外观偏好；关闭窗口后本次更改可能丢失。');
    }
  };
  return {
    theme,
    collapsed,
    warning,
    setTheme: (value: Theme) => {
      setTheme(value);
      save('omm.theme', value);
    },
    toggleSidebar: () => {
      save('omm.sidebar', collapsed ? 'expanded' : 'collapsed');
      setCollapsed(!collapsed);
    },
  };
}
