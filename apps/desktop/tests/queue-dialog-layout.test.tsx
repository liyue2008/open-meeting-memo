import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, screen, within } from '@testing-library/react';
import { render } from './render';
import { expect, it } from 'vitest';
import { BridgeContext, Notices } from '../src/app/context';
import { TaskControls } from '../src/features/queue';
import { fixtureBridge, NativeFailure, taskFixture } from './fixtures';

it('features/queue keeps a cancellation failure inside the active confirmation dialog', async () => {
  const message = 'SyntheticCancellationFailure'.repeat(8);
  const fake = fixtureBridge({
    processing_cancel: () => {
      throw new NativeFailure({
        code: 'unavailable',
        reason: null,
        message,
        retryable: false,
        fieldErrors: [],
      });
    },
  });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <BridgeContext.Provider value={fake.bridge}>
        <Notices>
          <TaskControls task={taskFixture()} />
        </Notices>
      </BridgeContext.Provider>
    </QueryClientProvider>,
  );
  fireEvent.click(screen.getByRole('button', { name: '取消任务' }));
  fireEvent.click(screen.getByRole('button', { name: '确认取消' }));
  expect(await within(screen.getByRole('dialog')).findByRole('alert')).toHaveTextContent(message);
});
