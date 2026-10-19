import { createElement, useEffect, useState, type ReactNode } from 'react';
import {
  MantineProvider,
  useComputedColorScheme,
  useMantineColorScheme,
  type MantineColorScheme,
  type MantineColorSchemeManager,
} from '@mantine/core';
export type Theme = 'system' | 'light' | 'dark';
function scheme(value: string | null): MantineColorScheme | null {
  return value === 'system' ? 'auto' : value === 'light' || value === 'dark' ? value : null;
}
function colorSchemeManager(): MantineColorSchemeManager {
  let receivingStorage = false;
  let listener: ((event: StorageEvent) => void) | undefined;
  return {
    get: (fallback) => {
      try {
        return scheme(localStorage.getItem('omm.theme')) ?? fallback;
      } catch {
        return fallback;
      }
    },
    set: (value) => {
      if (!receivingStorage) localStorage.setItem('omm.theme', value === 'auto' ? 'system' : value);
    },
    clear: () => localStorage.removeItem('omm.theme'),
    subscribe: (update) => {
      listener = (event) => {
        if (event.key === 'omm.theme') {
          const next = scheme(event.newValue);
          if (next || event.newValue === null) {
            receivingStorage = true;
            try {
              update(next ?? 'auto');
            } finally {
              receivingStorage = false;
            }
          }
        }
      };
      window.addEventListener('storage', listener);
    },
    unsubscribe: () => {
      if (listener) window.removeEventListener('storage', listener);
    },
  };
}
export function AppearanceProvider({ children }: { children: ReactNode }) {
  const [manager] = useState(colorSchemeManager);
  // Official styles.css already contains the unmodified default theme variables.
  // Avoid runtime <style> injection under production's style-src 'self' policy.
  return createElement(
    MantineProvider,
    {
      defaultColorScheme: 'auto',
      colorSchemeManager: manager,
      withCssVariables: false,
      withGlobalClasses: false,
    },
    children,
  );
}
export function useAppearance() {
  const { colorScheme, setColorScheme } = useMantineColorScheme({ keepTransitions: true });
  const computed = useComputedColorScheme('light');
  const theme: Theme = colorScheme === 'auto' ? 'system' : colorScheme;
  const [warning, setWarning] = useState('');
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem('omm.sidebar') === 'collapsed';
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      const t = localStorage.getItem('omm.theme'),
        s = localStorage.getItem('omm.sidebar');
      if ((t !== null && !scheme(t)) || (s !== null && !['collapsed', 'expanded'].includes(s)))
        setWarning('外观偏好损坏，已使用默认布局。');
    } catch {
      setWarning('无法读取外观偏好；本次更改只在当前窗口生效。');
    }
    const storage = (event: StorageEvent) => {
      if (event.key === 'omm.sidebar') setCollapsed(event.newValue === 'collapsed');
    };
    window.addEventListener('storage', storage);
    return () => window.removeEventListener('storage', storage);
  }, []);
  useEffect(() => {
    document.documentElement.dataset.theme = computed;
  }, [computed]);
  useEffect(() => {
    document.documentElement.dataset.sidebar = collapsed ? 'collapsed' : 'expanded';
  }, [collapsed]);
  return {
    theme,
    collapsed,
    warning,
    setTheme: (value: Theme) => {
      try {
        setColorScheme(value === 'system' ? 'auto' : value);
      } catch {
        setWarning('无法保存外观偏好；关闭窗口后本次更改可能丢失。');
      }
    },
    toggleSidebar: () => {
      const next = !collapsed;
      setCollapsed(next);
      try {
        localStorage.setItem('omm.sidebar', next ? 'collapsed' : 'expanded');
      } catch {
        setWarning('无法保存外观偏好；关闭窗口后本次更改可能丢失。');
      }
    },
  };
}
