import type { MeetingStatus, TaskStatus } from '../contracts.generated';

// Meeting status describes the available result; transcript_committed can have no active task.
const meetingActivity: Record<MeetingStatus, boolean> = {
  recording: true,
  unsealed: true,
  queued: true,
  transcribing: true,
  transcript_committed: false,
  summarizing: true,
  completed: false,
  blocked: false,
  failed: false,
  interrupted: false,
  cancelled: false,
  empty: false,
  unknown: false,
};

// Task status describes pipeline work; transcript_committed still awaits the summary stage.
const taskActivity: Record<TaskStatus, boolean> = {
  queued: true,
  transcribing: true,
  transcript_committed: true,
  summarizing: true,
  completed: false,
  blocked: false,
  failed: false,
  interrupted: false,
  cancelled: false,
};

export function isActiveMeetingStatus(status: MeetingStatus | undefined): boolean {
  return status === undefined ? false : meetingActivity[status];
}

export function isActiveTaskStatus(status: TaskStatus): boolean {
  return taskActivity[status];
}
