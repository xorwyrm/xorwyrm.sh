import * as THREE from 'three';

// The planet: a sphere of squares that flip on at random, glow under the cursor,
// ripple where you click, and carry a wyrm crawling over the surface.

// Ripples: up to RIP at once, each lasts RIP_LIFE seconds.
const RIP = 8;
const RIP_LIFE = 4;
// The wyrm is a chain of WN points on the sphere, head first.
const WN = 28;
const WGAP = 0.05; // angle between neighbouring body points

const BG = new THREE.Color(0x0d0b1c);
const PN = 16000, PR = 5; // dense enough to read as a surface, sparse enough to see the squares

const vertex = /* glsl */ `
  attribute float seed;
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

    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_Position = projectionMatrix * mv;
    gl_PointSize = uScale * 0.085 / -mv.z;
    vec3 vn = normalize(normalMatrix * n);
    vFace = vn.z;
    vLight = max(0.0, dot(vn, normalize(vec3(-0.5, 0.6, 0.62))));
    float wob = 0.5 + 0.5 * sin(n.y * 3.0 + n.x * 2.0 + t * 0.15);
    float mint = max(step(0.82, fract(seed * 37.0)), smoothstep(0.3, 1.0, ring));
    vOn = mix(mix(vec3(0.54, 0.6, 0.96), vec3(0.62, 0.6, 0.94), wob), vec3(0.5, 0.85, 0.71), mint);
    if (vWyrm > 0.5) vOn = ws < 0.07 ? vec3(0.68, 1.0, 0.86) : mix(vec3(0.5, 0.88, 0.72), vec3(0.58, 0.54, 0.9), smoothstep(0.1, 1.0, ws));
  }
`;

const fragment = /* glsl */ `
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

export function startField(canvas: HTMLCanvasElement) {
  let renderer: THREE.WebGLRenderer;
  try {
    renderer = new THREE.WebGLRenderer({ canvas, antialias: true, powerPreference: 'low-power' });
  } catch {
    document.documentElement.classList.add('no-webgl');
    return;
  }
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.setClearColor(BG, 1);

  const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)');
  let time = 0;

  const pos = new Float32Array(PN * 3), seed = new Float32Array(PN);
  for (let i = 0; i < PN; i++) {
    const y = 1 - ((i + 0.5) / PN) * 2, r = Math.sqrt(1 - y * y), th = i * Math.PI * (3 - Math.sqrt(5));
    pos.set([Math.cos(th) * r * PR, y * PR, Math.sin(th) * r * PR], i * 3);
    seed[i] = Math.random();
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  geo.setAttribute('seed', new THREE.BufferAttribute(seed, 1));
  const U = {
    uTime: { value: 0 }, uScale: { value: 600 }, uHoverDir: { value: new THREE.Vector3(0, 0, 1) }, uHover: { value: 0 },
    uRip: { value: Array.from({ length: RIP }, () => new THREE.Vector4(0, 1, 0, -100)) },
    uWyrm: { value: Array.from({ length: WN }, () => new THREE.Vector3()) },
  };
  const planet = new THREE.Points(geo, new THREE.ShaderMaterial({ uniforms: U, vertexShader: vertex, fragmentShader: fragment }));
  planet.rotation.x = 0.35;
  const scene = new THREE.Scene();
  scene.add(planet);
  const cam = new THREE.PerspectiveCamera(45, 1, 0.1, 100);

  // ---------- input ----------
  let ri = 0, hovering = false;
  const ndc = new THREE.Vector2();
  const ray = new THREE.Raycaster();
  const sphere = new THREE.Sphere(new THREE.Vector3(), PR);
  const hit = new THREE.Vector3(), dir = new THREE.Vector3(), invQ = new THREE.Quaternion();

  // the point under the cursor, as a direction in the planet's own (rotating) frame
  function planetDir(out: THREE.Vector3) {
    ray.setFromCamera(ndc, cam);
    if (!ray.ray.intersectSphere(sphere, hit)) return null;
    invQ.copy(planet.quaternion).invert();
    return out.copy(hit).applyQuaternion(invQ).normalize();
  }
  function setPointer(e: PointerEvent | MouseEvent) {
    const r = canvas.getBoundingClientRect();
    ndc.set(((e.clientX - r.left) / r.width) * 2 - 1, -(((e.clientY - r.top) / r.height) * 2 - 1));
  }
  const overUi = (t: EventTarget | null) => !!(t as HTMLElement | null)?.closest?.('a, button, header, .main');

  document.addEventListener('pointermove', (e) => { setPointer(e); hovering = !overUi(e.target); });
  document.documentElement.addEventListener('pointerleave', () => { hovering = false; });
  document.addEventListener('click', (e) => {
    if (overUi(e.target)) return;
    setPointer(e);
    const d = planetDir(dir);
    if (d) U.uRip.value[ri++ % RIP].set(d.x, d.y, d.z, time);
  });

  // ---------- the wyrm ----------
  // The head crawls over the sphere, wandering, and steers back toward the side facing you
  // when it strays (or toward the cursor when you're over the planet). Every body point keeps
  // a fixed angle from the one ahead, so the body follows the head's path.
  const body = U.uWyrm.value;
  const head = body[0];
  const fwd = new THREE.Vector3(1, 0, 0);
  const front = new THREE.Vector3(); // the side facing you, in world space; set in resize()
  const goal = new THREE.Vector3(), side = new THREE.Vector3(), step = new THREE.Vector3();
  head.set(0, 0, 1);
  body.forEach((p, i) => p.set(-Math.sin(i * WGAP), 0, Math.cos(i * WGAP)));

  function turnToward(target: THREE.Vector3) {
    // the angle between the heading and the way to the target, around the head's normal
    goal.copy(target).addScaledVector(head, -target.dot(head));
    side.crossVectors(head, fwd);
    return Math.atan2(goal.dot(side), goal.dot(fwd));
  }

  function stepWyrm(dt: number, hoverDir: THREE.Vector3 | null) {
    invQ.copy(planet.quaternion).invert();
    // gentle wandering turns; it only seeks the cursor from a distance, so it sweeps past
    // and loops back in wide arcs instead of coiling on the spot
    let turn = 0.45 * Math.sin(time * 0.37) + 0.3 * Math.sin(time * 0.91 + 2);
    if (hoverDir && head.dot(hoverDir) < 0.94) turn = 3 * turnToward(hoverDir);
    else {
      const toFront = step.copy(front).applyQuaternion(invQ);
      if (head.dot(toFront) < 0.55) turn = 2.5 * turnToward(toFront);
    }
    const a = THREE.MathUtils.clamp(turn, -0.9, 0.9) * dt;
    side.crossVectors(head, fwd);
    fwd.multiplyScalar(Math.cos(a)).addScaledVector(side, Math.sin(a)).normalize();
    // move along the heading plus a slither, then keep the heading flat to the surface
    const s = 0.45 * Math.sin(time * 2.6);
    side.crossVectors(head, fwd);
    step.copy(fwd).multiplyScalar(Math.cos(s)).addScaledVector(side, Math.sin(s));
    head.addScaledVector(step, (hoverDir ? 0.3 : 0.22) * dt).normalize();
    fwd.addScaledVector(head, -fwd.dot(head)).normalize();
    for (let i = 1; i < WN; i++) {
      const p = body[i], q = body[i - 1];
      p.sub(q).normalize().multiplyScalar(WGAP).add(q).normalize();
    }
  }

  // ---------- resize ----------
  function resize() {
    const w = canvas.clientWidth, h = canvas.clientHeight;
    renderer.setSize(w, h, false);
    const tanHalf = Math.tan(THREE.MathUtils.degToRad(cam.fov / 2));
    U.uScale.value = (h * renderer.getPixelRatio()) / (2 * tanHalf);
    // wide: fully round on the right half. narrow: a horizon along the bottom
    let r: number, cx: number, cy: number;
    if (w >= 760 && w > h) {
      r = Math.min(0.36 * h, 0.21 * w);
      cx = Math.min(0.745 * w, 0.95 * w - r);
      cy = h / 2;
      front.set(-0.2, 0.25, 1).normalize();
    } else {
      r = 0.62 * w;
      cx = w / 2;
      cy = h + 0.45 * r;
      front.set(0, 0.8, 0.6).normalize();
    }
    // move the camera back until the sphere's outline is r px, then shift the view (not the
    // sphere) so it lands at cx, cy and stays perfectly round
    cam.position.z = PR / Math.sin(Math.atan((2 * r * tanHalf) / h));
    cam.setViewOffset(w, h, w / 2 - cx, h / 2 - cy, w, h);
  }
  new ResizeObserver(resize).observe(canvas);
  resize();

  // ---------- loop ----------
  let last = 0;
  const hoverDir = new THREE.Vector3(0, 0, 1);

  function frame(now: number) {
    const dt = Math.min(last ? (now - last) / 1000 : 0, 1 / 20) * (reduceMotion.matches ? 0.25 : 1);
    last = now;
    time += dt;
    U.uTime.value = time;

    planet.rotation.y += dt * (0.06 + ndc.x * 0.04);
    const d = hovering ? planetDir(dir) : null;
    if (d) hoverDir.lerp(d, 0.3).normalize();
    U.uHoverDir.value.copy(hoverDir);
    U.uHover.value += ((d ? 1 : 0) - U.uHover.value) * 0.1;
    stepWyrm(dt, d ? hoverDir : null);

    renderer.render(scene, cam);
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
}
