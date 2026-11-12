import { startField } from './field';

// Everything here is extra: the pages work without it (see global.css for the no-JS name toggle).
// With it, Astro's router swaps pages in place: this runs once, the canvas persists across
// swaps, and anything that touches the rest of the page re-syncs on astro:page-load.

// number keys switch pages
document.addEventListener('keydown', (e) => {
  if (e.metaKey || e.ctrlKey || e.altKey || e.repeat) return;
  if ((e.target as HTMLElement).closest('input, textarea, select, [contenteditable]')) return;
  const link = document.querySelector<HTMLAnchorElement>(`nav a[data-key="${e.key}"]`);
  if (link && !link.classList.contains('on')) link.click();
});

startField(document.getElementById('field') as HTMLCanvasElement);

// a project is one big link that you can also select text in (Project.astro). letting go after
// dragging out a selection in it would follow the link; don't. a plain click (no drag) or a key
// press still does. capture, so it runs before the router's click handler
let pressedAt: [number, number] | null = null;
document.addEventListener('pointerdown', (e) => { pressedAt = [e.clientX, e.clientY]; }, true);
document.addEventListener('click', (e) => {
  const card = (e.target as Element | null)?.closest?.('.proj');
  if (!card || e.detail === 0 || !pressedAt) return;
  const dragged = Math.hypot(e.clientX - pressedAt[0], e.clientY - pressedAt[1]) > 4;
  const sel = getSelection();
  if (dragged && sel && !sel.isCollapsed && sel.containsNode(card, true)) e.preventDefault();
}, true);

// a swap replaces <html>'s attributes with the new page's; carry ours over before it paints
// (data-name: the name's state, below. no-webgl: set by field.ts)
document.addEventListener('astro:before-swap', (e) => {
  const from = document.documentElement, to = e.newDocument.documentElement;
  to.classList.toggle('no-webgl', from.classList.contains('no-webgl'));
  if (from.dataset.name) to.dataset.name = from.dataset.name;
  else delete to.dataset.name;
});

// pixel words are snapped to whole screen pixels as they're parsed (snapWord, Window.astro). a
// swap's new page doesn't run those inline scripts again, and resizing or zooming changes the
// sizes, so snap them again then
declare global { interface Window { snapWord(w: HTMLElement): void } }
const snapWords = () => document.querySelectorAll<HTMLElement>('.word').forEach((w) => window.snapWord(w));
document.addEventListener('astro:after-swap', snapWords);
addEventListener('resize', snapWords);
// zooming, or moving to a screen with a different pixel density, doesn't always fire resize.
// the change event can come before devicePixelRatio has caught up, so wait a frame
function watchDensity() {
  matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener('change', () => {
    requestAnimationFrame(() => { snapWords(); watchDensity(); });
  }, { once: true });
}
watchDensity();

// "hi, i'm <binary>": on a first visit the bits rebuild themselves into pixel letters a moment
// after the page loads (CSS transitions, positions from PixelWord.astro). That's remembered;
// after it, clicking flips between the two (also remembered).
// The decoded state is data-name="bit" on <html>; Window.astro sets it before paint.
const NAME_KEY = 'name';
const DECODE_AFTER = 1000; // ms

// 'bit' or 'binary'; anything else (unset, or a value we didn't write) counts as a first visit
function storedName() {
  let v: string | null = null;
  try { v = localStorage.getItem(NAME_KEY); } catch { /* storage blocked */ }
  return v === 'bit' || v === 'binary' ? v : null;
}
function setName(decoded: boolean) {
  if (decoded) document.documentElement.dataset.name = 'bit';
  else delete document.documentElement.dataset.name;
  try { localStorage.setItem(NAME_KEY, decoded ? 'bit' : 'binary'); } catch { /* storage blocked */ }
}

// every pixel word (the name, the page titles) shares the one state. each word is a label for
// a hidden checkbox, which is what the no-JS toggle runs on; here it just reports the click
let decodeTimer = 0;
function setupWords() {
  clearTimeout(decodeTimer);
  const toggles = document.querySelectorAll<HTMLInputElement>('.word input');
  toggles.forEach((t) => t.addEventListener('change', () => {
    clearTimeout(decodeTimer);
    setName(document.documentElement.dataset.name !== 'bit');
  }));
  // first visit: decode on its own. someone who has switched it back to binary keeps binary
  if (toggles.length && storedName() === null) {
    decodeTimer = window.setTimeout(() => { if (storedName() === null) setName(true); }, DECODE_AFTER);
  }
}
document.addEventListener('astro:page-load', setupWords);
