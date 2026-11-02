import { navigate } from 'astro:transitions/client';
import { startField } from './field';

// Runs once. The canvas persists across page swaps; the rest of the page doesn't,
// so anything that touches it re-syncs on astro:page-load.

document.addEventListener('keydown', (e) => {
  if (e.metaKey || e.ctrlKey || e.altKey || e.repeat) return;
  if ((e.target as HTMLElement).closest('input, textarea, select, [contenteditable]')) return;
  const link = document.querySelector<HTMLAnchorElement>(`nav a[data-key="${e.key}"]`);
  if (link && !link.classList.contains('on')) navigate(link.href);
});

startField(document.getElementById('field') as HTMLCanvasElement);

// "hi, i'm <binary>": on a first visit the bits rebuild themselves into pixel letters a moment
// after the page loads (CSS transitions, positions from PixelWord.astro). That's remembered;
// after it, clicking flips between the two (also remembered).
// The decoded state is data-name="bit" on <html>; Window.astro sets it before paint.
const NAME_KEY = 'name';
const DECODE_AFTER = 1000; // ms
let decodeTimer = 0;

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
// page swaps replace <html>'s attributes, so put the state back before the new page paints
document.addEventListener('astro:after-swap', () => {
  if (storedName() === 'bit') document.documentElement.dataset.name = 'bit';
});

// every pixel word (the name, the page titles) shares the one state
function setupHello() {
  const words = document.querySelectorAll<HTMLElement>('.hello');
  words.forEach((hello) => {
    hello.addEventListener('click', () => {
      clearTimeout(decodeTimer);
      setName(document.documentElement.dataset.name !== 'bit');
    });
  });
  // first visit: decode on its own. someone who has switched it back to binary keeps binary
  if (words.length && storedName() === null) {
    clearTimeout(decodeTimer);
    decodeTimer = window.setTimeout(() => { if (storedName() === null) setName(true); }, DECODE_AFTER);
  }
}

document.addEventListener('astro:page-load', setupHello);
