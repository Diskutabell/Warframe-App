// Short messages in the corner: what a change did, or why it failed.

export type Toast = { id: number; text: string; err: boolean };

export const toasts = $state<Toast[]>([]);
let next = 0;

export function toast(text: string, err = false) {
  const id = ++next;
  toasts.push({ id, text, err });
  setTimeout(() => {
    const i = toasts.findIndex((t) => t.id === id);
    if (i >= 0) toasts.splice(i, 1);
  }, err ? 7000 : 3200);
}
