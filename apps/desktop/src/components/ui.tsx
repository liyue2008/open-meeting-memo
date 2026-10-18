import { useEffect, useRef, useState, type ReactNode } from 'react';
import {
  Accordion,
  Alert,
  Badge,
  Button,
  Group,
  Loader,
  Modal as MantineModal,
  Stack,
  Text,
  Title,
} from '@mantine/core';
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
    <Alert color="red" role="alert" className="notice" title="操作未完成">
      <Stack gap="sm">
        <Text className="notice-copy">{errorText(error)}</Text>
        {retry && (
          <Button variant="default" onClick={retry}>
            重试读取
          </Button>
        )}
      </Stack>
    </Alert>
  );
}
export function Loading() {
  return (
    <Group role="status" justify="center" p="xl">
      <Loader size="sm" />
      <Text c="dimmed">正在读取…</Text>
    </Group>
  );
}
export function Status({ status }: { status: MeetingStatus | TaskStatus }) {
  return (
    <Badge
      variant="light"
      color={
        status === 'completed'
          ? 'green'
          : status === 'failed' || status === 'blocked'
            ? 'red'
            : status === 'recording'
              ? 'orange'
              : 'blue'
      }
    >
      {STATUS[status]}
    </Badge>
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
    <Group justify="space-between" align="flex-start" wrap="wrap" mb="lg" className="page-heading">
      <div className="heading-copy">
        <Title order={1}>{title}</Title>
        {text && (
          <Text c="dimmed" mt="xs">
            {text}
          </Text>
        )}
      </div>
      {children && (
        <Group gap="sm" className="button-row">
          {children}
        </Group>
      )}
    </Group>
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
  const [dialog, setDialog] = useState<HTMLDivElement | null>(null);
  const lastFocused = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (!dialog) return;
    // Mantine traps Tab. WebKit additionally drops disabled focus to
    // BODY: recover only an invalidated target, without stealing valid field focus.
    const observer = new MutationObserver(() => {
      const last = lastFocused.current;
      const active = document.activeElement;
      if (
        last &&
        [...document.querySelectorAll('[role="dialog"]')].at(-1) === dialog &&
        (!dialog.contains(last) || last.matches(':disabled')) &&
        (active === document.body || active === last)
      )
        dialog.focus({ preventScroll: true });
    });
    observer.observe(dialog, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['disabled'],
    });
    return () => observer.disconnect();
  }, [dialog]);
  // Callers conditionally mount dialogs; Mantine's opened-transition return
  // cannot run when the whole component unmounts. Restore that trigger here.
  useEffect(() => {
    const previous = document.activeElement;
    return () => {
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus({ preventScroll: true });
    };
  }, []);
  return (
    <MantineModal.Root
      opened
      onClose={() => {
        if (!busy) close();
      }}
      closeOnEscape={!busy}
      closeOnClickOutside={false}
      returnFocus={false}
      lockScroll={false}
      keepMounted={false}
      size="lg"
      transitionProps={{ duration: 0 }}
    >
      <MantineModal.Overlay />
      <MantineModal.Content
        ref={setDialog}
        aria-busy={busy}
        onFocusCapture={(event) => {
          if (event.target instanceof HTMLElement) lastFocused.current = event.target;
        }}
      >
        <MantineModal.Header>
          <MantineModal.Title>{title}</MantineModal.Title>
          <MantineModal.CloseButton disabled={busy} aria-label="关闭对话框" />
        </MantineModal.Header>
        <MantineModal.Body>
          <Stack gap="md">{children}</Stack>
        </MantineModal.Body>
      </MantineModal.Content>
    </MantineModal.Root>
  );
}
export function Disclosure({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <Accordion variant="separated" className={className}>
      <Accordion.Item value="details">
        <Accordion.Control>{title}</Accordion.Control>
        <Accordion.Panel>{children}</Accordion.Panel>
      </Accordion.Item>
    </Accordion>
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
