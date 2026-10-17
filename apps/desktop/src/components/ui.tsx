import { useEffect, useRef, useId, type ReactNode } from 'react';
import { errorText } from '../bridge/client';
import type { ArtifactStatus, MeetingStatus, TaskStatus } from '../contracts.generated';
export const STATUS: Record<MeetingStatus | TaskStatus, string> = {
  recording: '录音中',
  unsealed: '尚未封存',
  queued: '排队中',
  transcribing: '转写中',
  transcript_committed: '逐字稿已提交',
  summarizing: '生成纪要',
  completed: '已完成',
  blocked: '需处理',
  failed: '失败',
  interrupted: '已中断',
  cancelled: '已取消',
  empty: '无有效音频',
  unknown: '状态未知',
};
export const ARTIFACT_STATUS: Record<ArtifactStatus, string> = {
  missing: '尚无产物',
  partial: '部分内容',
  committed: '已提交',
  invalid: '校验失败',
  historical: '历史产物',
};
export const SOURCES = {
  lark: '飞书',
  manual: '手动录音',
  imported: '导入混音',
  legacy: '历史录音',
  unknown: '来源未知',
};
export function ErrorNotice({ error, retry }: { error: unknown; retry?: () => void }) {
  return (
    <div role="alert" className="notice error">
      {errorText(error)} {retry && <button onClick={retry}>重试读取</button>}
    </div>
  );
}
export function Loading() {
  return (
    <p role="status" className="empty-state">
      正在读取…
    </p>
  );
}
export function Status({ status }: { status: MeetingStatus | TaskStatus }) {
  return (
    <span
      className={`badge ${status === 'completed' ? 'success' : status === 'failed' || status === 'blocked' ? 'warning' : ''}`}
    >
      {STATUS[status]}
    </span>
  );
}
export function Heading({
  title,
  text,
  children,
}: {
  title: string;
  text?: string;
  children?: ReactNode;
}) {
  return (
    <div className="page-heading">
      <div>
        <div className="eyebrow">OPEN MEETING MEMO</div>
        <h1>{title}</h1>
        {text && <p>{text}</p>}
      </div>
      <div className="button-row">{children}</div>
    </div>
  );
}
export function Modal({
  title,
  children,
  close,
  busy = false,
}: {
  title: string;
  children: ReactNode;
  close: () => void;
  busy?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const previous = document.activeElement;
    const dialog = ref.current;
    dialog?.showModal();
    return () => {
      dialog?.close();
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      onCancel={(e) => {
        e.preventDefault();
        if (!busy) close();
      }}
    >
      <div className="modal-header">
        <h2 id={titleId}>{title}</h2>
        <button aria-label="关闭对话框" disabled={busy} onClick={close}>
          ×
        </button>
      </div>
      {children}
    </dialog>
  );
}
export function Icon({ name }: { name: string }) {
  const paths: Record<string, ReactNode> = {
    library: (
      <>
        <path d="M12 5.5C9 3.5 6 3.5 3 4.5v15c3-1 6-1 9 1 3-2 6-2 9-1v-15c-3-1-6-1-9 1Z M12 5.5v15 M6 8h3 M15 8h3 M6 11h3 M15 11h3" />
      </>
    ),
    queue: (
      <>
        <rect x="3" y="3" width="18" height="18" rx="2" />
        <path d="M7 7.5h.01 M11 7.5h6 M7 12h.01 M11 12h6 M7 16.5h.01 M11 16.5h6" />
      </>
    ),
    settings: (
      <>
        <circle cx="12" cy="12" r="6" />
        <circle cx="12" cy="12" r="2.3" />
        <path d="M12 3v3 M12 18v3 M3 12h3 M18 12h3 M5.6 5.6l2.1 2.1 M16.3 16.3l2.1 2.1 M5.6 18.4l2.1-2.1 M16.3 7.7l2.1-2.1" />
      </>
    ),
    diagnostics: (
      <>
        <rect x="3" y="3" width="18" height="18" rx="3" />
        <path d="M6 12h3l2-4 3 8 2-4h2" />
      </>
    ),
    help: (
      <>
        <circle cx="12" cy="12" r="9" />
        <path d="M9.5 9a2.5 2.5 0 0 1 5 .2c0 1.8-2.5 1.8-2.5 3.8 M12 16.5h.01" />
      </>
    ),
    collapse: (
      <>
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="M9 4v16 M15 9l-3 3 3 3" />
      </>
    ),
  };
  return (
    <svg
      className="nav-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {paths[name] ?? paths.help}
    </svg>
  );
}
export function Logo() {
  return (
    <span className="brand-mark">
      <svg
        className="brand-wordmark"
        viewBox="0 0 46 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <path
          data-letter="o"
          d="M7 6.5h1.5L11 9v4.5c0 2.2-1.8 4-4 4s-4-1.8-4-4v-3c0-2.2 1.8-4 4-4Z"
        />
        <path
          data-letter="m"
          d="M15 17.5v-10 M15 10.5c0-4.8 6-4.8 6 0v7 M21 10.5c0-4.8 6-4.8 6 0v7 M31 17.5v-10 M31 10.5c0-4.8 6-4.8 6 0v7 M37 10.5c0-4.8 6-4.8 6 0v7"
        />
      </svg>
    </span>
  );
}
