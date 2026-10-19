import { act, fireEvent, screen } from '@testing-library/react';
import { vi } from 'vitest';

// JSDOM has no layout. Supply only the input track's synthetic geometry to
// exercise Mantine's real mouse handling; page geometry is checked in WK.
export async function seekSlider(milliseconds: number) {
  const thumb = screen.getByRole('slider', { name: '会议时间定位' });
  const track = thumb.closest('.mantine-Slider-trackContainer') as HTMLElement;
  const bounds = vi
    .spyOn(track, 'getBoundingClientRect')
    .mockReturnValue(new DOMRect(0, 0, 1000, 12));
  try {
    await act(async () => {
      fireEvent.mouseDown(track, {
        clientX: (milliseconds / Number(thumb.getAttribute('aria-valuemax'))) * 1000,
        clientY: 6,
        button: 0,
      });
      await new Promise(requestAnimationFrame);
      fireEvent.mouseUp(document);
    });
  } finally {
    bounds.mockRestore();
  }
}
