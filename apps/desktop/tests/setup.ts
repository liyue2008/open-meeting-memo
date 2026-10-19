import '@testing-library/jest-dom/vitest';
import { afterEach, vi } from 'vitest';
import { cleanup } from '@testing-library/react';
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
  delete document.documentElement.dataset.theme;
  delete document.documentElement.dataset.sidebar;
  delete document.documentElement.dataset.mantineColorScheme;
  systemDark(false);
});
Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
  configurable: true,
  value: () => undefined,
});
Object.defineProperty(HTMLMediaElement.prototype, 'play', {
  configurable: true,
  value: () => {
    throw new Error('Audio playback is forbidden in frontend tests');
  },
});
Object.defineProperty(HTMLMediaElement.prototype, 'load', {
  configurable: true,
  value: () => undefined,
});
Object.defineProperty(HTMLMediaElement.prototype, 'pause', {
  configurable: true,
  value: () => undefined,
});
const media = new EventTarget();
Object.defineProperty(media, 'matches', { configurable: true, writable: true, value: false });
Object.assign(media, {
  media: '(prefers-color-scheme: dark)',
  onchange: null,
  addListener: () => undefined,
  removeListener: () => undefined,
  dispatchEvent: EventTarget.prototype.dispatchEvent,
});
Object.defineProperty(window, 'matchMedia', { configurable: true, value: () => media });
export function systemDark(dark: boolean) {
  Object.defineProperty(media, 'matches', { configurable: true, value: dark });
  const event = new Event('change');
  Object.defineProperty(event, 'matches', { value: dark });
  Object.defineProperty(event, 'media', { value: '(prefers-color-scheme: dark)' });
  media.dispatchEvent(event);
}

// JSDOM has no layout engine: size observation is inert here; real geometry is
// verified in the private WK fixture, not synthesized into passing rectangles.
vi.stubGlobal(
  'ResizeObserver',
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
);
