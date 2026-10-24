import { reactive } from "vue";

export interface CtxItem {
  label: string;
  icon?: "play" | "album" | "artist" | "export" | "delete" | "playlist" | "edit" | "save";
  danger?: boolean;
  action: () => void;
}

interface CtxState {
  x: number;
  y: number;
  items: CtxItem[] | null;
}

export const ctx = reactive<CtxState>({ x: 0, y: 0, items: null });

const MENU_W = 190;
const ITEM_H = 32;
const PAD = 8;

/** 在鼠标位置打开右键菜单(自动防出屏) */
export function openCtx(e: MouseEvent, items: CtxItem[]): void {
  e.preventDefault();
  e.stopPropagation();
  const h = items.length * ITEM_H + PAD * 2;
  let x = e.clientX;
  let y = e.clientY;
  if (x + MENU_W > window.innerWidth - 8) x = window.innerWidth - MENU_W - 8;
  if (y + h > window.innerHeight - 8) y = window.innerHeight - h - 8;
  ctx.x = Math.max(8, x);
  ctx.y = Math.max(8, y);
  ctx.items = items;
}

export function closeCtx(): void {
  ctx.items = null;
}
