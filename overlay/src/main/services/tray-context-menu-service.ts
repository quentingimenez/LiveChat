import type { Menu, Tray } from 'electron';

export interface TrayPopupPosition {
  x: number;
  y: number;
}

export function getTrayPopupPosition(tray: Tray | null): TrayPopupPosition | null {
  if (!tray || typeof tray.getBounds !== 'function') {
    return null;
  }

  const bounds = tray.getBounds();
  if (!bounds) {
    return null;
  }

  const hasXAndWidth = Number.isFinite(bounds.x) && Number.isFinite(bounds.width);
  const hasY = Number.isFinite(bounds.y);
  if (!hasXAndWidth || !hasY) {
    return null;
  }

  const x = Math.round(bounds.x + bounds.width / 2);
  const y = Math.max(0, Math.round(bounds.y - 2));

  return { x, y };
}

export function popUpTrayContextMenu(tray: Tray | null, menu: Menu | null): void {
  if (!tray || !menu) {
    return;
  }

  const position = getTrayPopupPosition(tray);
  if (position) {
    tray.popUpContextMenu(menu, position);
    return;
  }

  tray.popUpContextMenu(menu);
}

export function closeTrayContextMenu(tray: Tray | null): void {
  if (!tray || typeof tray.closeContextMenu !== 'function') {
    return;
  }

  tray.closeContextMenu();
}
