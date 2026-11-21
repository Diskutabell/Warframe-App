// WebKitGTK on Wayland moves a whole wheel click (about 50-100 px) in one frame, which reads as a
// stutter. Wheel clicks glide here instead: each adds to where the scroll is heading, and the
// view eases there over a few frames. Touchpads send many small steps and keep the native path.
// Only the box under the pointer that can move that way glides: an inner list scrolls itself
// before the page does, as the browser would.

const heading = new Map<HTMLElement, number>();

function step(el: HTMLElement) {
  const target = heading.get(el)!;
  const left = target - el.scrollTop;
  if (Math.abs(left) < 0.5) {
    el.scrollTop = target;
    heading.delete(el);
    return;
  }
  const before = el.scrollTop;
  el.scrollTop += left * 0.28 + Math.sign(left) * 0.5;
  // An edge (or another script) stopped it: the target is out of reach.
  if (el.scrollTop === before) {
    heading.delete(el);
    return;
  }
  requestAnimationFrame(() => step(el));
}

/** The nearest box from `from` up that scrolls vertically and has room to go `down` (or up). */
function scroller(from: Element | null, down: boolean): HTMLElement | null {
  for (let el = from as HTMLElement | null; el && el !== document.body; el = el.parentElement) {
    const style = getComputedStyle(el);
    if (!/(auto|scroll)/.test(style.overflowY) || el.scrollHeight <= el.clientHeight) continue;
    const at = heading.get(el) ?? el.scrollTop;
    if (down ? at < el.scrollHeight - el.clientHeight - 0.5 : at > 0.5) return el;
  }
  return null;
}

export function glide(e: WheelEvent) {
  if (e.ctrlKey || e.shiftKey || Math.abs(e.deltaY) < 30 || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
  const el = scroller(e.target as Element, e.deltaY > 0);
  if (!el) return;
  e.preventDefault();
  const delta = e.deltaMode === 1 ? e.deltaY * 18 : e.deltaY;
  const from = heading.get(el) ?? el.scrollTop;
  const target = Math.max(0, Math.min(el.scrollHeight - el.clientHeight, from + delta));
  const gliding = heading.has(el);
  heading.set(el, target);
  if (!gliding) requestAnimationFrame(() => step(el));
}
