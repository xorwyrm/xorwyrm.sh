import { startField } from './field';

// Everything here is extra: the pages work without it (see global.css for the no-JS name toggle).

// number keys switch pages
document.addEventListener('keydown', (e) => {
  if (e.metaKey || e.ctrlKey || e.altKey || e.repeat) return;
  if ((e.target as HTMLElement).closest('input, textarea, select, [contenteditable]')) return;
  const link = document.querySelector<HTMLAnchorElement>(`nav a[data-key="${e.key}"]`);
  if (link && !link.classList.contains('on')) link.click();
});

startField(document.getElementById('field') as HTMLCanvasElement);

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
const toggles = document.querySelectorAll<HTMLInputElement>('.word input');
let decodeTimer = 0;
toggles.forEach((t) => t.addEventListener('change', () => {
  clearTimeout(decodeTimer);
  setName(document.documentElement.dataset.name !== 'bit');
}));
// first visit: decode on its own. someone who has switched it back to binary keeps binary
if (toggles.length && storedName() === null) {
  decodeTimer = window.setTimeout(() => { if (storedName() === null) setName(true); }, DECODE_AFTER);
}
