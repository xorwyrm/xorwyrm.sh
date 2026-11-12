// The planet, after the site's field.ts: a sphere of squares that flip on at random, glow
// under the cursor, ripple where you click, and carry a wyrm crawling over the surface.
// The site draws it with WebGL; here each square is a braille dot (term::Dots), eight to a
// cell, and the shader's colour maths runs per square on the CPU. There are fewer squares
// than on the site, as many as fit with a gap between them, so the ripples are drawn a bit
// wider to still read at this size.

use crate::term::{Dots, Rgb};

type V = [f32; 3];
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
/// a * j + b * k
fn mix(a: V, j: f32, b: V, k: f32) -> V {
    [a[0] * j + b[0] * k, a[1] * j + b[1] * k, a[2] * j + b[2] * k]
}
fn norm(a: V) -> V {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let l = if l > 0.0 { l } else { 1.0 };
    [a[0] / l, a[1] / l, a[2] / l]
}
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn step(edge: f32, x: f32) -> f32 {
    if x >= edge { 1.0 } else { 0.0 }
}
fn fract(x: f32) -> f32 {
    x - x.floor()
}
fn vmix(a: V, b: V, t: f32) -> V {
    mix(a, 1.0 - t, b, t)
}

// Ripples: up to RIP at once, each lasts RIP_LIFE seconds.
const RIP: usize = 32;
const RIP_LIFE: f32 = 4.0;
// The wyrm is a chain of WN points on the sphere, head first.
const WN: usize = 28;
const WGAP: f32 = 0.05; // angle between neighbouring body points
const TILT: f32 = 0.35; // the planet leans toward you by this much (radians)
/// Braille dots per subpixel, each way: the planet is drawn at twice the resolution of
/// everything else.
const DOTS: f32 = 2.0;
/// Squares are this many dots apart in the middle of the planet.
const SPACING: f32 = 1.7;
/// The site's squares are about this far apart, in radians.
const SITE_SPACING: f32 = 0.028;

const BG: V = [0.051, 0.043, 0.11];

/// Where the planet sits, in subpixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
    /// the side facing you, which the wyrm keeps to
    pub front: [f32; 3],
}

pub struct Planet {
    /// points on the sphere and their seeds
    pts: Vec<(V, f32)>,
    built_for: f32,
    view: View,
    time: f32,
    spin: f32,
    rot: [f32; 9],
    body: [V; WN],
    fwd: V,
    rip: [(V, f32); RIP],
    ri: usize,
    hover: f32,
    hover_dir: V,
    /// the pointer, in subpixels, while it's over the planet
    pointer: Option<(f32, f32)>,
    /// the pointer across the whole screen, -1 to 1: it nudges the spin, as on the site
    ndc_x: f32,
}

impl Planet {
    pub fn new() -> Planet {
        let mut body = [[0.0; 3]; WN];
        for (i, b) in body.iter_mut().enumerate() {
            let a = i as f32 * WGAP;
            *b = [-a.sin(), 0.0, a.cos()];
        }
        let mut p = Planet {
            pts: Vec::new(),
            built_for: 0.0,
            view: View { cx: 0.0, cy: 0.0, r: 0.0, front: [0.0, 0.0, 1.0] },
            time: 0.0,
            spin: 0.0,
            rot: [0.0; 9],
            body,
            fwd: [1.0, 0.0, 0.0],
            rip: [([0.0, 1.0, 0.0], -100.0); RIP],
            ri: 0,
            hover: 0.0,
            hover_dir: [0.0, 0.0, 1.0],
            pointer: None,
            ndc_x: 0.0,
        };
        p.set_rotation();
        p
    }

    pub fn set_view(&mut self, v: View) {
        self.view = View { front: norm(v.front), ..v };
        // as many squares as fit SPACING apart; rebuilt when the size changes much
        let r = v.r * DOTS;
        if (r - self.built_for).abs() > self.built_for * 0.08 {
            self.built_for = r;
            let n = ((4.0 * std::f32::consts::PI * r * r) / (SPACING * SPACING)).clamp(200.0, 60000.0) as usize;
            self.pts = lattice(n);
        }
    }

    /// The pointer moved: `over` is where it is in subpixels, if it's over the planet and not
    /// the page's text. `ndc_x` is its place across the screen, -1 to 1.
    pub fn set_pointer(&mut self, over: Option<(f32, f32)>, ndc_x: f32) {
        self.pointer = over;
        self.ndc_x = ndc_x;
    }

    /// A click at (x, y) in subpixels: a ripple, if it's on the planet.
    pub fn click(&mut self, x: f32, y: f32) {
        if let Some(d) = self.dir_at(x, y) {
            self.rip[self.ri % RIP] = (d, self.time);
            self.ri += 1;
        }
    }

    // the rotation: lean toward you by TILT, then spin. by columns, as the site's shader has it
    fn set_rotation(&mut self) {
        let (sa, ca) = TILT.sin_cos();
        let (sb, cb) = self.spin.sin_cos();
        self.rot = [cb, sa * sb, -ca * sb, 0.0, ca, sa, sb, -sa * cb, ca * cb];
    }
    /// a direction in the planet's own frame, turned to face the world
    fn to_world(&self, p: V) -> V {
        let r = &self.rot;
        [
            r[0] * p[0] + r[3] * p[1] + r[6] * p[2],
            r[1] * p[0] + r[4] * p[1] + r[7] * p[2],
            r[2] * p[0] + r[5] * p[1] + r[8] * p[2],
        ]
    }
    /// a world direction in the planet's own (rotating) frame
    fn to_planet(&self, w: V) -> V {
        let r = &self.rot;
        [
            r[0] * w[0] + r[1] * w[1] + r[2] * w[2],
            r[3] * w[0] + r[4] * w[1] + r[5] * w[2],
            r[6] * w[0] + r[7] * w[1] + r[8] * w[2],
        ]
    }
    /// the point under (x, y), as a direction in the planet's frame
    fn dir_at(&self, x: f32, y: f32) -> Option<V> {
        let v = &self.view;
        if v.r <= 0.0 {
            return None;
        }
        let (wx, wy) = ((x - v.cx) / v.r, -(y - v.cy) / v.r);
        let d2 = wx * wx + wy * wy;
        (d2 < 1.0).then(|| self.to_planet([wx, wy, (1.0 - d2).sqrt()]))
    }

    pub fn update(&mut self, dt: f32) {
        self.time += dt;
        self.spin += dt * (0.06 + self.ndc_x * 0.04);
        self.set_rotation();
        let d = self.pointer.and_then(|(x, y)| self.dir_at(x, y));
        // the site eases these per frame at 60fps; the same, at whatever rate this runs
        let frames = dt * 60.0;
        if let Some(d) = d {
            let k = 1.0 - 0.7f32.powf(frames);
            self.hover_dir = norm(mix(self.hover_dir, 1.0 - k, d, k));
        }
        let target = if d.is_some() { 1.0 } else { 0.0 };
        self.hover += (target - self.hover) * (1.0 - 0.9f32.powf(frames));
        let seek = d.map(|_| self.hover_dir);
        self.step_wyrm(dt, seek);
    }

    // ---------- the wyrm ----------
    // The head crawls over the sphere, wandering, and steers back toward the side facing you
    // when it strays (or toward the cursor when you're over the planet). Every body point keeps
    // a fixed angle from the one ahead, so the body follows the head's path.

    fn turn_toward(&self, target: V) -> f32 {
        // the angle between the heading and the way to the target, around the head's normal
        let head = self.body[0];
        let goal = mix(target, 1.0, head, -dot(target, head));
        let side = cross(head, self.fwd);
        dot(goal, side).atan2(dot(goal, self.fwd))
    }

    fn step_wyrm(&mut self, dt: f32, seek: Option<V>) {
        let t = self.time;
        // gentle wandering turns; it only seeks the cursor from a distance, so it sweeps past
        // and loops back in wide arcs instead of coiling on the spot
        let mut turn = 0.45 * (t * 0.37).sin() + 0.3 * (t * 0.91 + 2.0).sin();
        match seek {
            Some(s) if dot(self.body[0], s) < 0.94 => turn = 3.0 * self.turn_toward(s),
            _ => {
                let to_front = self.to_planet(self.view.front);
                if dot(self.body[0], to_front) < 0.55 {
                    turn = 2.5 * self.turn_toward(to_front);
                }
            }
        }
        let a = turn.clamp(-0.9, 0.9) * dt;
        let head = self.body[0];
        self.fwd = norm(mix(self.fwd, a.cos(), cross(head, self.fwd), a.sin()));
        // move along the heading plus a slither, then keep the heading flat to the surface
        let s = 0.45 * (t * 2.6).sin();
        let step = mix(self.fwd, s.cos(), cross(head, self.fwd), s.sin());
        self.body[0] = norm(mix(head, 1.0, step, if seek.is_some() { 0.3 } else { 0.22 } * dt));
        self.fwd = norm(mix(self.fwd, 1.0, self.body[0], -dot(self.fwd, self.body[0])));
        for i in 1..WN {
            let q = self.body[i - 1];
            self.body[i] = norm(mix(q, 1.0, norm(mix(self.body[i], 1.0, q, -1.0)), WGAP));
        }
    }

    /// How far along the wyrm's body n is (0 at the head, 1 at the tail), if it's on it.
    fn wyrm_at(&self, n: V, thinnest: f32) -> Option<f32> {
        let mut ws = 2.0f32;
        for i in 0..WN - 1 {
            let a = self.body[i];
            let ba = mix(self.body[i + 1], 1.0, a, -1.0);
            let pa = mix(n, 1.0, a, -1.0);
            let k = (dot(pa, ba) / dot(ba, ba).max(1e-6)).clamp(0.0, 1.0);
            let s = (i as f32 + k) / (WN - 1) as f32;
            // widest at the head and thin at the tail
            let r = ((0.06 + (0.022 - 0.06) * s) + 0.025 * (1.0 - smoothstep(0.0, 0.1, s))).max(thinnest);
            let off = mix(pa, 1.0, ba, -k);
            if dot(off, off) < r * r {
                ws = ws.min(s);
            }
        }
        (ws < 1.5).then_some(ws)
    }

    /// Draws the planet in braille dots: twice the subpixels, each way.
    pub fn draw(&self, c: &mut Dots) {
        let v = View { cx: self.view.cx * DOTS, cy: self.view.cy * DOTS, r: self.view.r * DOTS, ..self.view };
        if v.r < 4.0 {
            return;
        }
        let t = self.time;
        // how much coarser this is than the site: ripples widen to match, and the wyrm's tail
        // stays at least a couple of dots wide
        let res = (SPACING / v.r) / SITE_SPACING;
        let ring_w = 0.07 * (res * 0.7).max(1.0);
        let thinnest = 1.1 / v.r;
        let light_dir = norm([-0.5, 0.6, 0.62]);
        let rips: Vec<(V, f32)> =
            self.rip.iter().copied().filter(|&(_, t0)| t - t0 >= 0.0 && t - t0 <= RIP_LIFE).collect();

        // the colour of a square, as the site's shaders work it out, and how much it matters
        // when it shares a cell: the wyrm, then lit squares, then the rest
        let shade = |n: V, w: V, seed: f32, wyrm: Option<f32>| -> (Rgb, u8) {
            let b = step(0.96, fract(seed * 91.7 + (t * 0.25 + seed * 13.0).floor() * 0.618_034));
            // a soft glow around the cursor, brightest in the middle
            let g = smoothstep(0.32, 0.0, dot(n, self.hover_dir).clamp(-1.0, 1.0).acos());
            let glow = g * g * self.hover;
            let mut ring = 0.0;
            for &(r, t0) in &rips {
                let age = t - t0;
                let d = (dot(n, r).clamp(-1.0, 1.0).acos() - age * 0.9) / ring_w;
                ring += (-d * d).exp() * (1.0 - age / RIP_LIFE);
            }
            let is_wyrm = if wyrm.is_some() { 1.0 } else { 0.0 };
            let lit = b.max(step(0.35, ring)).max(is_wyrm);
            let light = dot(w, light_dir).max(0.0);
            let wob = 0.5 + 0.5 * (n[1] * 3.0 + n[0] * 2.0 + t * 0.15).sin();
            let mint = step(0.82, fract(seed * 37.0)).max(smoothstep(0.3, 1.0, ring));
            let mut on = vmix(vmix([0.54, 0.6, 0.96], [0.62, 0.6, 0.94], wob), [0.5, 0.85, 0.71], mint);
            if let Some(ws) = wyrm {
                on = if ws < 0.07 {
                    [0.68, 1.0, 0.86]
                } else {
                    vmix([0.5, 0.88, 0.72], [0.58, 0.54, 0.9], smoothstep(0.1, 1.0, ws))
                };
            }
            // lit from the upper left: unlit squares fade out on the shadow side, lit ones stay
            // visible, and the wyrm stays brightest
            let off = vmix(BG, [0.47, 0.45, 0.75], 0.06 + 0.5 * light);
            let k = (0.35 + 0.7 * light).min(1.0);
            let k = k + (1.0 - k) * is_wyrm * 0.55;
            let on = [on[0] * k, on[1] * k, on[2] * k];
            let col = vmix(off, on, lit);
            let hi = [col[0].max(0.66), col[1].max(0.88), col[2].max(0.9)];
            let col = vmix(col, hi, glow * 0.95);
            let rank = if wyrm.is_some() { 2 } else if lit > 0.5 { 1 } else { 0 };
            (Rgb::f(col[0], col[1], col[2]), rank)
        };

        // the squares, nearest on top
        let mut z = vec![f32::MIN; (c.w * c.h).max(0) as usize];
        for &(n, seed) in &self.pts {
            let w = self.to_world(n);
            if w[2] < 0.05 {
                continue;
            }
            let (x, y) = ((v.cx + w[0] * v.r).floor() as i32, (v.cy - w[1] * v.r).floor() as i32);
            if x < 0 || y < 0 || x >= c.w || y >= c.h {
                continue;
            }
            let zi = (y * c.w + x) as usize;
            if w[2] <= z[zi] {
                continue;
            }
            z[zi] = w[2];
            let (col, rank) = shade(n, w, seed, None);
            c.set(x, y, col, rank);
        }

        // the wyrm, solid: every dot on its body, within the box around it
        let reach = (0.06 + 0.025) * v.r + 2.0;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &b in &self.body {
            let w = self.to_world(b);
            let (x, y) = (v.cx + w[0] * v.r, v.cy - w[1] * v.r);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let (x0, x1) = (((x0 - reach).floor() as i32).max(0), ((x1 + reach).ceil() as i32).min(c.w - 1));
        let (y0, y1) = (((y0 - reach).floor() as i32).max(0), ((y1 + reach).ceil() as i32).min(c.h - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (wx, wy) = ((x as f32 + 0.5 - v.cx) / v.r, -(y as f32 + 0.5 - v.cy) / v.r);
                let d2 = wx * wx + wy * wy;
                if d2 >= 1.0 {
                    continue;
                }
                let w = [wx, wy, (1.0 - d2).sqrt()];
                if w[2] < 0.05 {
                    continue;
                }
                let n = self.to_planet(w);
                if let Some(ws) = self.wyrm_at(n, thinnest) {
                    let (col, rank) = shade(n, w, 0.0, Some(ws));
                    c.set(x, y, col, rank);
                }
            }
        }
    }
}

/// Points on a Fibonacci sphere, with seeds from the same generator as the site's.
fn lattice(n: usize) -> Vec<(V, f32)> {
    let mut rnd: u32 = 0x9e37_79b9;
    let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    (0..n)
        .map(|i| {
            let y = 1.0 - ((i as f32 + 0.5) / n as f32) * 2.0;
            let r = (1.0 - y * y).sqrt();
            // in f64: the angle runs to tens of thousands of radians
            let th = (i as f64 * golden).rem_euclid(std::f64::consts::TAU) as f32;
            rnd = rnd.wrapping_add(0x6d2b_79f5);
            let mut z = (rnd ^ (rnd >> 15)).wrapping_mul(1 | rnd);
            z ^= z.wrapping_add((z ^ (z >> 7)).wrapping_mul(61 | z));
            let seed = (z ^ (z >> 14)) as f32 / 4_294_967_296.0;
            ([th.cos() * r, y, th.sin() * r], seed)
        })
        .collect()
}
