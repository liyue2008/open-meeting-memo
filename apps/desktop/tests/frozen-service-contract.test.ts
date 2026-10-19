import { describe, expect, it } from 'vitest';
import * as validators from '../src/bridge/validators.generated.js';
import { sample } from './fixtures';
const response = (data: unknown) => ({ schemaVersion: 1, requestId: 'fixture-request-1', data });
describe('frozen desktop service UI response validation', () => {
  it('accepts nullable saved preferences without inventing a recording state', () => {
    const record = {
      ...sample('record_status'),
      state: 'unknown',
      autoRecord: null,
      recordingConsent: null,
    };
    expect(validators.record_status_response(response(record))).toBe(true);
    expect(
      validators.config_get_response(
        response({ ...sample('config_get'), autoRecord: null, recordingConsent: null }),
      ),
    ).toBe(true);
    expect(validators.record_status_response(response({ ...record, state: 'idle_unknown' }))).toBe(
      false,
    );
    expect(validators.record_status_response(response({ ...record, autoRecord: 'false' }))).toBe(
      false,
    );
  });
  it('accepts bounded effects and text guidance but refuses unknown reasons and executable-shaped objects', () => {
    const plan = {
      ...sample('service_plan'),
      effects: ['Do not start; preserve configuration'],
      guidance: '<script>plain text only</script>',
    };
    expect(validators.service_plan_response(response(plan))).toBe(true);
    expect(
      validators.service_plan_response(
        response({
          ...plan,
          state: 'unknown',
          canApply: false,
          planToken: null,
          expiresAtMs: null,
          reasons: ['external_unload_required'],
        }),
      ),
    ).toBe(true);
    expect(
      validators.service_plan_response(response({ ...plan, reasons: ['unknown_future_reason'] })),
    ).toBe(false);
    expect(
      validators.service_plan_response(response({ ...plan, guidance: { command: 'execute' } })),
    ).toBe(false);
    expect(
      validators.service_plan_response(response({ ...plan, effects: [{ command: 'execute' }] })),
    ).toBe(false);
  });
});
