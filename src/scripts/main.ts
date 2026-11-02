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

// "hi, i'm <binary>": the first hover (or tap) rebuilds the bits into pixel letters
// spelling the name (CSS transitions, positions from PixelWord.astro). That's remembered;
// after it, clicking flips between the two (also remembered).
// The decoded state is data-name="bit" on <html>; Window.astro sets it before paint.
const NAME_KEY = 'name';
const canHover = matchMedia('(hover: hover)');

function storedName() {
  try { return localStorage.getItem(NAME_KEY); } catch { return null; }
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
  document.querySelectorAll<HTMLElement>('.hello').forEach((hello) => {
    hello.addEventListener('mouseenter', () => {
      if (!canHover.matches || storedName() !== null || document.documentElement.dataset.name) return;
      setName(true);
    });
    hello.addEventListener('click', () => {
      setName(document.documentElement.dataset.name !== 'bit');
    });
  });
}

document.addEventListener('astro:page-load', setupHello);
