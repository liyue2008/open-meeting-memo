import fs from 'node:fs';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ErrorNotice, Heading } from '../src/components/ui';
import { Notices, useNotice } from '../src/app/context';

const token = 'SyntheticLongToken'.repeat(14);
describe('components/ui and app/context long content layout seams', () => {
  it('gives heading text and error text independent shrinkable regions without truncating content', () => {
    render(
      <>
        <Heading title={token} text={token}>
          <button>操作</button>
        </Heading>
        <ErrorNotice error={new Error(token)} retry={() => undefined} />
      </>,
    );
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(token);
    expect(screen.getByRole('heading', { level: 1 }).parentElement).toHaveClass('heading-copy');
    expect(screen.getByRole('alert').querySelector('.notice-copy')).toHaveTextContent(token);
    expect(screen.getByRole('button', { name: '重试读取' })).toBeEnabled();
    const css = fs.readFileSync('src/styles.css', 'utf8');
    expect(css).toMatch(/\.heading-copy\s*\{[^}]*min-width:\s*0/);
    expect(css).toMatch(/\.notice-copy\s*\{[^}]*overflow-wrap:\s*anywhere/);
  });
  it('wraps bare warnings and settings validation text at their owning boundaries', () => {
    const css = fs.readFileSync('src/styles.css', 'utf8');
    expect(css).toMatch(/\.notice\s*\{[^}]*overflow-wrap:\s*anywhere/);
    expect(css).toMatch(/\.settings-grid,\s*\.settings-save\s*\{[^}]*overflow-wrap:\s*anywhere/);
    expect(css).toMatch(
      /\.settings-save:has\(\[role='alert'\]\),\s*\.settings-save:has\(details\[open\]\)\s*\{[^}]*position:\s*static/,
    );
  });
  it('renders a visible dismissible status without relying on a demo-only class toggle', () => {
    function Trigger() {
      const notice = useNotice();
      return <button onClick={() => notice(token)}>提示</button>;
    }
    render(
      <Notices>
        <Trigger />
      </Notices>,
    );
    fireEvent.click(screen.getByRole('button', { name: '提示' }));
    expect(screen.getByRole('status')).toHaveClass('visible');
    expect(screen.getByRole('status')).toHaveAttribute('aria-live', 'polite');
    fireEvent.click(screen.getByRole('button', { name: '关闭提示' }));
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    const css = fs.readFileSync('src/styles.css', 'utf8');
    expect(css).toMatch(/#toast\s*\{[^}]*pointer-events:\s*auto/);
  });
});
