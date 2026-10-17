import { describe, expect, it } from 'vitest';
import { DESKTOP_BUDGETS } from '../src/contracts.generated';
describe('foundation frozen budgets', () => {
  it('bounds pages, bridge data and media memory independently', () => {
    expect(DESKTOP_BUDGETS.defaultPageSize).toBe(25);
    expect(DESKTOP_BUDGETS.maxPageSize).toBe(100);
    expect(DESKTOP_BUDGETS.bridgeBodyBytes).toBe(1_048_576);
    expect(DESKTOP_BUDGETS.mediaResponseBytes).toBe(8_388_608);
  });
});
