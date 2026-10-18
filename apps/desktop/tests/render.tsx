import {
  render as testingRender,
  renderHook as testingRenderHook,
  type RenderOptions,
  type RenderHookOptions,
} from '@testing-library/react';
import type { ReactNode } from 'react';
import { AppearanceProvider } from '../src/app/preferences';
const wrapper = ({ children }: { children: ReactNode }) => (
  <AppearanceProvider>{children}</AppearanceProvider>
);
export function render(ui: ReactNode, options?: RenderOptions) {
  return testingRender(ui, { wrapper, ...options });
}
export function renderHook<Result, Props>(
  hook: (props: Props) => Result,
  options?: RenderHookOptions<Props>,
) {
  return testingRenderHook(hook, { wrapper, ...options });
}
