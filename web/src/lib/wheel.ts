// WebKitGTK on Wayland moves a whole wheel click (about 50-100 px) in one frame, which reads as a
// stutter. Wheel clicks glide here instead: each adds to where the scroll is heading, and the
// view eases there over a few frames. Touchpads send many small steps and keep the native path.
// Only the box under the pointer that can move that way glides: an inner list scrolls itself
// before the page does, as the browser would.

/** Where each gliding box is heading, and where the glide left it last frame. */
const heading = new Map<HTMLElement, { target: number; at: number }>();

function step(el: HTMLElement) {
  const g = heading.get(el);
  if (!g) return;
  // Moved by something else since (a new page set it to the top, the scrollbar, a key): that wins.
  if (Math.abs(el.scrollTop - g.at) > 1) {
    heading.delete(el);
    return;
  }
  const left = g.target - el.scrollTop;
  if (Math.abs(left) < 0.5) {
    el.scrollTop = g.target;
    heading.delete(el);
    return;
  }
  const before = el.scrollTop;
  el.scrollTop += left * 0.28 + Math.sign(left) * 0.5;
  // An edge stopped it: the target is out of reach.
  if (el.scrollTop === before) {
    heading.delete(el);
    return;
  }
  g.at = el.scrollTop;
  requestAnimationFrame(() => step(el));
}

/** The nearest box from `from` up that scrolls vertically and has room to go `down` (or up). */
function scroller(from: Element | null, down: boolean): HTMLElement | null {
  for (let el = from as HTMLElement | null; el && el !== document.body; el = el.parentElement) {
    const style = getComputedStyle(el);
    if (!/(auto|scroll)/.test(style.overflowY) || el.scrollHeight <= el.clientHeight) continue;
    const at = heading.get(el)?.target ?? el.scrollTop;
    if (down ? at < el.scrollHeight - el.clientHeight - 0.5 : at > 0.5) return el;
  }
  return null;
}

export function glide(e: WheelEvent) {
  // In pixels: some mice report lines (about 3 a click) or whole pages.
  const delta = e.deltaMode === 1 ? e.deltaY * 18 : e.deltaMode === 2 ? e.deltaY * window.innerHeight * 0.9 : e.deltaY;
  if (e.ctrlKey || e.shiftKey || Math.abs(delta) < 30 || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
  const el = scroller(e.target as Element, delta > 0);
  if (!el) return;
  e.preventDefault();
  const g = heading.get(el);
  const target = Math.max(0, Math.min(el.scrollHeight - el.clientHeight, (g?.target ?? el.scrollTop) + delta));
  if (g) g.target = target;
  else {
    heading.set(el, { target, at: el.scrollTop });
    requestAnimationFrame(() => step(el));
  }
}
