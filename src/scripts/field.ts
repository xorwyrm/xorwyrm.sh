// The planet: a sphere of squares that flip on at random, glow under the cursor,
// ripple where you click, and carry a wyrm crawling over the surface.
// Plain WebGL: one draw call of points, with the look worked out in the shaders.

// Ripples: up to RIP at once, each lasts RIP_LIFE seconds.
const RIP = 32;
const RIP_LIFE = 4;
// The wyrm is a chain of WN points on the sphere, head first.
const WN = 28;
const WGAP = 0.05; // angle between neighbouring body points

const PN = 16000, PR = 5; // dense enough to read as a surface, sparse enough to see the squares
const TAN_HALF = Math.tan((22.5 * Math.PI) / 180); // 45° vertical field of view
const TILT = 0.35; // the planet leans toward you by this much (radians)
// pages are separate documents, so the planet's state is handed from one to the next
const STATE_KEY = 'field';

const vertex = /* glsl */ `
  attribute vec3 position; attribute float seed;
  // uRot: the planet's rotation. uProj: x and y scale, then the shift that moves the view
  // (not the sphere) so the planet sits off-centre and stays round
  uniform mat3 uRot; uniform float uCamZ; uniform vec4 uProj;
  uniform float uTime; uniform float uScale; uniform vec3 uHoverDir; uniform float uHover;
  uniform vec4 uRip[${RIP}]; uniform vec3 uWyrm[${WN}];
  varying float vLit; varying float vFace; varying float vLight; varying float vWyrm; varying float vGlow; varying vec3 vOn;
  void main() {
    vec3 n = normalize(position);
    float t = uTime;
    float b = step(0.96, fract(seed * 91.7 + floor(t * 0.25 + seed * 13.0) * 0.6180339));
    // a soft glow around the cursor, brightest in the middle
    float g = smoothstep(0.32, 0.0, acos(clamp(dot(n, uHoverDir), -1.0, 1.0)));
    vGlow = g * g * uHover;
    float ring = 0.0;
    for (int i = 0; i < ${RIP}; i++) {
      vec4 r = uRip[i]; float age = t - r.w;
      if (age < 0.0 || age > ${RIP_LIFE.toFixed(1)}) continue;
      float d = (acos(clamp(dot(n, r.xyz), -1.0, 1.0)) - age * 0.9) / 0.07;
      ring += exp(-d * d) * (1.0 - age / ${RIP_LIFE.toFixed(1)});
    }
    // the wyrm: squares within its body, widest at the head and thin at the tail
    float ws = 2.0;
    for (int i = 0; i < ${WN - 1}; i++) {
      vec3 a = uWyrm[i], ba = uWyrm[i + 1] - a, pa = n - a;
      float k = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
      float s = (float(i) + k) / ${(WN - 1).toFixed(1)};
      float r = mix(0.06, 0.022, s) + 0.025 * (1.0 - smoothstep(0.0, 0.1, s));
      if (length(pa - ba * k) < r) ws = min(ws, s);
    }
    vWyrm = step(ws, 1.5);
    vLit = max(max(b, step(0.35, ring)), vWyrm);

    // camera at (0, 0, uCamZ) looking down -z; near 0.1, far 100
    vec3 eye = uRot * position - vec3(0.0, 0.0, uCamZ);
    gl_Position = vec4(uProj.x * eye.x + uProj.z * eye.z, uProj.y * eye.y + uProj.w * eye.z, -1.002 * eye.z - 0.2002, -eye.z);
    gl_PointSize = uScale * 0.085 / -eye.z;
    vec3 vn = uRot * n;
    vFace = vn.z;
    vLight = max(0.0, dot(vn, normalize(vec3(-0.5, 0.6, 0.62))));
    float wob = 0.5 + 0.5 * sin(n.y * 3.0 + n.x * 2.0 + t * 0.15);
    float mint = max(step(0.82, fract(seed * 37.0)), smoothstep(0.3, 1.0, ring));
    vOn = mix(mix(vec3(0.54, 0.6, 0.96), vec3(0.62, 0.6, 0.94), wob), vec3(0.5, 0.85, 0.71), mint);
    if (vWyrm > 0.5) vOn = ws < 0.07 ? vec3(0.68, 1.0, 0.86) : mix(vec3(0.5, 0.88, 0.72), vec3(0.58, 0.54, 0.9), smoothstep(0.1, 1.0, ws));
  }
`;

const fragment = /* glsl */ `
  precision mediump float;
  varying float vLit; varying float vFace; varying float vLight; varying float vWyrm; varying float vGlow; varying vec3 vOn;
  void main() {
    if (vFace < 0.05) discard;
    vec2 q = abs(gl_PointCoord - 0.5);
    // the wyrm's squares are bigger, so it reads as one body; squares swell a little in the glow
    if (max(q.x, q.y) > mix(0.3, 0.43, vWyrm) + 0.09 * vGlow) discard;
    // lit from the upper left: unlit squares fade out on the shadow side, lit ones stay visible,
    // and the wyrm stays brightest
    vec3 bg = vec3(0.051, 0.043, 0.11);
    vec3 off = mix(bg, vec3(0.47, 0.45, 0.75), 0.06 + 0.5 * vLight);
    vec3 on = vOn * mix(min(1.0, 0.35 + 0.7 * vLight), 1.0, vWyrm * 0.55);
    vec3 col = mix(off, on, vLit);
    col = mix(col, max(col, vec3(0.66, 0.88, 0.9)), vGlow * 0.95);
    gl_FragColor = vec4(col, 1.0);
  }
`;

type V = [number, number, number];
const dot = (a: V, b: V) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: V, b: V): V => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
// a * j + b * k
const mix = (a: V, j: number, b: V, k: number): V => [a[0] * j + b[0] * k, a[1] * j + b[1] * k, a[2] * j + b[2] * k];
const norm = (a: V): V => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };

function program(gl: WebGLRenderingContext) {
  const p = gl.createProgram()!;
  for (const [type, src] of [[gl.VERTEX_SHADER, vertex], [gl.FRAGMENT_SHADER, fragment]] as const) {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    gl.attachShader(p, s);
  }
  gl.linkProgram(p);
  return gl.getProgramParameter(p, gl.LINK_STATUS) ? p : null;
}

export function startField(canvas: HTMLCanvasElement) {
  const noWebgl = () => document.documentElement.classList.add('no-webgl');
  // transparent, so a frame that isn't drawn yet shows the page behind it, not black
  const ctx = canvas.getContext('webgl', { antialias: true, alpha: true, powerPreference: 'low-power' });
  const prog = ctx && program(ctx);
  if (!ctx || !prog) return noWebgl();
  const gl = ctx;
  canvas.addEventListener('webglcontextlost', noWebgl);
  gl.useProgram(prog);
  gl.clearColor(0, 0, 0, 0);
  gl.enable(gl.DEPTH_TEST);
  gl.depthFunc(gl.LEQUAL);

  // points on a Fibonacci sphere. the seeds come from a fixed generator, so every page
  // draws the same planet
  const pos = new Float32Array(PN * 3), seed = new Float32Array(PN);
  let rnd = 0x9e3779b9;
  for (let i = 0; i < PN; i++) {
    const y = 1 - ((i + 0.5) / PN) * 2, r = Math.sqrt(1 - y * y), th = i * Math.PI * (3 - Math.sqrt(5));
    pos.set([Math.cos(th) * r * PR, y * PR, Math.sin(th) * r * PR], i * 3);
    rnd = (rnd + 0x6d2b79f5) | 0;
    let z = Math.imul(rnd ^ (rnd >>> 15), 1 | rnd);
    z ^= z + Math.imul(z ^ (z >>> 7), 61 | z);
    seed[i] = ((z ^ (z >>> 14)) >>> 0) / 4294967296;
  }
  for (const [name, data, size] of [['position', pos, 3], ['seed', seed, 1]] as const) {
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, name);
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, size, gl.FLOAT, false, 0, 0);
  }
  const u = (name: string) => gl.getUniformLocation(prog, name);
  const U = {
    rot: u('uRot'), camZ: u('uCamZ'), proj: u('uProj'), time: u('uTime'), scale: u('uScale'),
    hoverDir: u('uHoverDir'), hover: u('uHover'), rip: u('uRip'), wyrm: u('uWyrm'),
  };

  const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)');
  let time = 0;
  let spin = 0; // the planet's turn around its own axis
  let last = 0; // last frame's timestamp

  // the rotation: lean toward you by TILT, then spin. stored by columns for the shader
  const R = new Float32Array(9);
  const sa = Math.sin(TILT), ca = Math.cos(TILT);
  function setRotation() {
    const sb = Math.sin(spin), cb = Math.cos(spin);
    R.set([cb, sa * sb, -ca * sb, 0, ca, sa, sb, -sa * cb, ca * cb]);
  }
  // a world direction in the planet's own (rotating) frame
  const toPlanet = (w: V): V => [
    R[0] * w[0] + R[1] * w[1] + R[2] * w[2],
    R[3] * w[0] + R[4] * w[1] + R[5] * w[2],
    R[6] * w[0] + R[7] * w[1] + R[8] * w[2],
  ];

  // ---------- input ----------
  let ri = 0, hovering = false, hover = 0;
  let ndc = [0, 0];
  let hoverDir: V = [0, 0, 1];
  const rip = new Float32Array(RIP * 4);
  for (let i = 0; i < RIP; i++) rip.set([0, 1, 0, -100], i * 4);

  // ---------- camera ----------
  let camZ = 20, sx = 1, sy = 1, shiftX = 0, shiftY = 0;
  let front: V = [0, 0, 1]; // the side facing you, in world space; set in resize()
  function resize() {
    const w = canvas.clientWidth, h = canvas.clientHeight, dpr = Math.min(devicePixelRatio, 2);
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    gl.viewport(0, 0, canvas.width, canvas.height);
    // wide: fully round on the right half. narrow: a horizon along the bottom
    let r: number, cx: number, cy: number;
    if (w >= 760 && w > h) {
      r = Math.min(0.36 * h, 0.21 * w);
      cx = Math.min(0.745 * w, 0.95 * w - r);
      cy = h / 2;
      front = norm([-0.2, 0.25, 1]);
    } else {
      r = 0.62 * w;
      cx = w / 2;
      cy = h + 0.45 * r;
      front = norm([0, 0.8, 0.6]);
    }
    // move the camera back until the sphere's outline is r px, then shift the view so its
    // centre lands at cx, cy
    camZ = PR / Math.sin(Math.atan((2 * r * TAN_HALF) / h));
    sx = h / (w * TAN_HALF);
    sy = 1 / TAN_HALF;
    shiftX = (w - 2 * cx) / w;
    shiftY = (2 * cy - h) / h;
    gl.uniform1f(U.camZ, camZ);
    gl.uniform4f(U.proj, sx, sy, shiftX, shiftY);
    gl.uniform1f(U.scale, (h * dpr) / (2 * TAN_HALF));
    draw(); // resizing clears the canvas
  }

  // the point under the cursor, as a direction in the planet's frame
  function planetDir(): V | null {
    const d = norm([(ndc[0] + shiftX) / sx, (ndc[1] + shiftY) / sy, -1]);
    const b = camZ * d[2], disc = b * b - (camZ * camZ - PR * PR);
    if (disc < 0) return null;
    const t = -b - Math.sqrt(disc);
    return toPlanet(norm([d[0] * t, d[1] * t, camZ + d[2] * t]));
  }
  function setPointer(e: PointerEvent | MouseEvent) {
    const r = canvas.getBoundingClientRect();
    ndc = [((e.clientX - r.left) / r.width) * 2 - 1, -(((e.clientY - r.top) / r.height) * 2 - 1)];
  }
  const overUi = (t: EventTarget | null) => !!(t as HTMLElement | null)?.closest?.('a, button, header, .main');

  document.addEventListener('pointermove', (e) => { setPointer(e); hovering = !overUi(e.target); });
  document.documentElement.addEventListener('pointerleave', () => { hovering = false; });
  document.addEventListener('click', (e) => {
    if (overUi(e.target)) return;
    setPointer(e);
    const d = planetDir();
    if (d) rip.set([...d, time], (ri++ % RIP) * 4);
  });

  // ---------- the wyrm ----------
  // The head crawls over the sphere, wandering, and steers back toward the side facing you
  // when it strays (or toward the cursor when you're over the planet). Every body point keeps
  // a fixed angle from the one ahead, so the body follows the head's path.
  const body: V[] = Array.from({ length: WN }, (_, i) => [-Math.sin(i * WGAP), 0, Math.cos(i * WGAP)]);
  let fwd: V = [1, 0, 0];

  function turnToward(target: V) {
    // the angle between the heading and the way to the target, around the head's normal
    const head = body[0], goal = mix(target, 1, head, -dot(target, head)), side = cross(head, fwd);
    return Math.atan2(dot(goal, side), dot(goal, fwd));
  }

  function stepWyrm(dt: number, seek: V | null) {
    // gentle wandering turns; it only seeks the cursor from a distance, so it sweeps past
    // and loops back in wide arcs instead of coiling on the spot
    let turn = 0.45 * Math.sin(time * 0.37) + 0.3 * Math.sin(time * 0.91 + 2);
    if (seek && dot(body[0], seek) < 0.94) turn = 3 * turnToward(seek);
    else {
      const toFront = toPlanet(front);
      if (dot(body[0], toFront) < 0.55) turn = 2.5 * turnToward(toFront);
    }
    const a = Math.max(-0.9, Math.min(0.9, turn)) * dt;
    const head = body[0];
    fwd = norm(mix(fwd, Math.cos(a), cross(head, fwd), Math.sin(a)));
    // move along the heading plus a slither, then keep the heading flat to the surface
    const s = 0.45 * Math.sin(time * 2.6);
    const step = mix(fwd, Math.cos(s), cross(head, fwd), Math.sin(s));
    body[0] = norm(mix(head, 1, step, (seek ? 0.3 : 0.22) * dt));
    fwd = norm(mix(fwd, 1, body[0], -dot(fwd, body[0])));
    for (let i = 1; i < WN; i++) {
      const q = body[i - 1];
      body[i] = norm(mix(q, 1, norm(mix(body[i], 1, q, -1)), WGAP));
    }
  }

  // ---------- state across pages ----------
  function save() {
    try {
      sessionStorage.setItem(STATE_KEY, JSON.stringify({ time, spin, hover, hovering, fwd, ndc, hoverDir, body: body.flat(), rip: [...rip] }));
    } catch { /* storage blocked */ }
  }
  function restore() {
    let s;
    try { s = JSON.parse(sessionStorage.getItem(STATE_KEY) ?? 'null'); } catch { return; }
    const nums = (v: unknown, n: number): v is number[] => Array.isArray(v) && v.length === n && v.every(Number.isFinite);
    // ignore anything that isn't exactly what save() writes
    if (!s || !nums([s.time, s.spin, s.hover], 3) || !nums(s.fwd, 3) || !nums(s.ndc, 2) || !nums(s.hoverDir, 3)
      || !nums(s.body, WN * 3) || !nums(s.rip, RIP * 4)) return;
    ({ time, spin, hover, ndc } = s);
    fwd = s.fwd as V;
    hoverDir = s.hoverDir as V;
    hovering = s.hovering === true;
    for (let i = 0; i < WN; i++) body[i] = s.body.slice(i * 3, i * 3 + 3);
    rip.set(s.rip);
  }
  addEventListener('pagehide', save);
  // back/forward can bring this page back from memory: carry on from the page just left
  addEventListener('pageshow', (e) => { if (e.persisted) { restore(); last = 0; draw(); } });
  restore();

  // ---------- loop ----------
  function draw() {
    setRotation();
    gl.uniformMatrix3fv(U.rot, false, R);
    gl.uniform1f(U.time, time);
    gl.uniform3fv(U.hoverDir, hoverDir);
    gl.uniform1f(U.hover, hover);
    gl.uniform4fv(U.rip, rip);
    gl.uniform3fv(U.wyrm, body.flat());
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.drawArrays(gl.POINTS, 0, PN);
  }

  function frame(now: number) {
    const dt = Math.min(last ? (now - last) / 1000 : 0, 1 / 20) * (reduceMotion.matches ? 0.25 : 1);
    last = now;
    time += dt;
    spin += dt * (0.06 + ndc[0] * 0.04);
    setRotation();
    const d = hovering ? planetDir() : null;
    if (d) hoverDir = norm(mix(hoverDir, 0.7, d, 0.3));
    hover += ((d ? 1 : 0) - hover) * 0.1;
    stepWyrm(dt, d ? hoverDir : null);
    draw();
    requestAnimationFrame(frame);
  }

  // drawn straight away (resize draws), so the first paint already has the planet
  new ResizeObserver(resize).observe(canvas);
  resize();
  requestAnimationFrame(frame);
}
