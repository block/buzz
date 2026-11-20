// Free-roaming bee swarm. Each bee wanders the WHOLE viewport on a slowly-curving
// per-bee heading, while a boids-style separation force keeps neighbours apart so
// the field never collapses into clumps (the failure mode of the old position-
// based velocity field). Soft edge containment steers bees back inward, and a
// final hard clamp guarantees they never leave the frame. `seed` decorrelates
// each bee's heading drift so the swarm fans out instead of marching in lockstep.
export type Bee = {
  x: number;
  y: number;
  vx: number;
  vy: number;
  heading: number; // current wander direction (radians)
  seed: number;
  homeX?: number;
  homeY?: number;
};

// Cursor repel. The force is intentionally visible but restrained; earlier
// feedback asked for the strong repulsion to be cut by half.
const REPEL_RADIUS = 380;
const REPEL_FORCE = 1600;

// Wander: bees cruise at ~SPEED px/s; velocity steers toward the heading-aligned
// target at rate STEER (1/s). TURN_RATE is the heading's angular drift amplitude.
const SPEED = 58;
const STEER = 2.4;
const TURN_RATE = 1.15;
const MIN_SPEED_FACTOR = 0.46;
const SPEED_SWELL = 0.32;
const DART_WINDOW = 0.18;
const DART_SPEED_FACTOR = 1.1;
const DART_STEER = 11;

// Boids separation — inverse-distance push from neighbours inside SEP_RADIUS.
// This is the anti-clump force; without it free roaming converges into blobs.
const SEP_RADIUS = 66;
const SEP_FORCE = 1300;

// Edge avoidance: within MARGIN of any edge we both turn the bee's HEADING
// inward (EDGE_TURN, rad/s — so it actually leaves the boundary instead of
// parking against it) and add a gentle inward velocity nudge (EDGE_FORCE) as a
// backstop. Heading steering is the important half: a velocity-only push left
// bees whose heading pointed outward standing off in the corners, which read as
// a clump.
const MARGIN = 110;
const EDGE_FORCE = 900;
const EDGE_TURN = 3.2;

// Recovery/home force: bees still wander freely, but they gently prefer a
// center-biased resting scatter inside the visible viewport. The pull ramps up
// in the bleed area so cursor-repelled bees recover instead of loitering
// off-screen.
const HOME_PULL = 0.18;
const OFFSCREEN_HOME_PULL = 0.9;
const HOME_PADDING_X = 0.12;
const HOME_PADDING_Y = 0.14;

// Light damping keeps the integration stable without killing the cruise speed
// (the STEER term regulates speed back toward SPEED after any impulse).
const DAMP = 0.97;
const TAU = Math.PI * 2;

// R2 low-discrepancy sequence (Roberts 2018): far more even 2D coverage than two
// independent golden-ratio sequences, which line up into diagonal lattices. Used
// for the initial scatter so the field starts evenly spread.
const G = 1.324717957244746;
const A1 = 1 / G;
const A2 = 1 / (G * G);
const frac = (n: number) => n - Math.floor(n);

export function spawnBees(
  n: number,
  w: number,
  h: number,
  visibleInset = 0,
): Bee[] {
  const bees: Bee[] = [];
  const inset = Math.max(0, Math.min(visibleInset, w / 2, h / 2));
  const visibleW = Math.max(1, w - inset * 2);
  const visibleH = Math.max(1, h - inset * 2);
  for (let i = 0; i < n; i++) {
    const gx = frac(0.5 + A1 * (i + 1));
    const gy = frac(0.5 + A2 * (i + 1));
    const seed = frac((i + 1) * 0.7548776662);
    const homeX =
      inset + (HOME_PADDING_X + gx * (1 - HOME_PADDING_X * 2)) * visibleW;
    const homeY =
      inset + (HOME_PADDING_Y + gy * (1 - HOME_PADDING_Y * 2)) * visibleH;
    bees.push({
      x: gx * w,
      y: gy * h,
      vx: 0,
      vy: 0,
      // Spread initial headings around the circle via the seed.
      heading: seed * TAU,
      seed,
      homeX,
      homeY,
    });
  }
  return bees;
}

export function stepBees(
  bees: Bee[],
  pointer: { x: number; y: number } | null,
  dt: number,
  w: number,
  h: number,
  repel: boolean,
  time = 0,
  visibleInset = 0,
): Bee[] {
  return bees.map((b, i) => {
    let { x, y, vx, vy, heading } = b;
    const { seed } = b;
    const homeX = b.homeX ?? w / 2;
    const homeY = b.homeY ?? h / 2;
    const inset = Math.max(0, Math.min(visibleInset, w / 2, h / 2));
    const visibleLeft = inset;
    const visibleRight = w - inset;
    const visibleTop = inset;
    const visibleBottom = h - inset;

    // Heading curves smoothly over time. Two incommensurate low-frequency terms
    // make the turn rate itself drift, so a bee traces big slow loops across the
    // frame rather than wobbling around one fixed bearing (which biased the swarm
    // toward whatever direction it spawned facing).
    heading +=
      (Math.sin(time * (0.27 + seed * 0.4) + seed * TAU) +
        0.6 * Math.sin(time * (0.13 + seed * 0.11) + seed * 7)) *
      TURN_RATE *
      dt;

    // Edge avoidance — turn the heading toward the inward direction near any
    // edge so the bee curves away from the wall (proportional to how deep into
    // the margin it is). This is what keeps bees from piling into the corners.
    let inX = 0,
      inY = 0;
    if (x < MARGIN) inX = 1 - x / MARGIN;
    else if (x > w - MARGIN) inX = -(1 - (w - x) / MARGIN);
    if (y < MARGIN) inY = 1 - y / MARGIN;
    else if (y > h - MARGIN) inY = -(1 - (h - y) / MARGIN);
    if (inX !== 0 || inY !== 0) {
      const strength = Math.min(1, Math.hypot(inX, inY));
      const want = Math.atan2(inY, inX);
      const diff = ((want - heading + Math.PI * 3) % TAU) - Math.PI;
      heading += diff * EDGE_TURN * strength * dt;
    }

    // Steer velocity toward the heading-aligned cruise velocity. The target
    // speed swells slowly, with short seeded burst windows layered in, so some
    // bees occasionally dart and then settle back down.
    const speedSwell =
      1 + Math.sin(time * (0.42 + seed * 0.28) + seed * TAU) * SPEED_SWELL;
    const dartPhase = (time * (0.24 + seed * 0.2) + seed * 5.37) % 1;
    const dartAmount =
      dartPhase < DART_WINDOW
        ? Math.sin((dartPhase / DART_WINDOW) * Math.PI)
        : 0;
    const visibleW = Math.max(1, visibleRight - visibleLeft);
    const dartSpeed = visibleW * DART_SPEED_FACTOR * dartAmount;
    const targetSpeed = Math.max(
      SPEED * MIN_SPEED_FACTOR,
      SPEED * speedSwell + dartSpeed,
    );
    const desiredVx = Math.cos(heading) * targetSpeed;
    const desiredVy = Math.sin(heading) * targetSpeed;
    const steer = STEER + DART_STEER * dartAmount;
    vx += (desiredVx - vx) * steer * dt;
    vy += (desiredVy - vy) * steer * dt;

    // Separation — push away from every neighbour inside SEP_RADIUS.
    for (let j = 0; j < bees.length; j++) {
      if (j === i) continue;
      const o = bees[j];
      const dx = x - o.x,
        dy = y - o.y;
      const d = Math.hypot(dx, dy);
      if (d > 0 && d < SEP_RADIUS) {
        const f = (SEP_FORCE * (1 - d / SEP_RADIUS)) / d;
        vx += dx * f * dt;
        vy += dy * f * dt;
      }
    }

    // Cursor repel — same impulse shape as separation, larger radius/force.
    if (repel && pointer) {
      const dx = x - pointer.x,
        dy = y - pointer.y;
      const dist = Math.hypot(dx, dy);
      if (dist > 0 && dist < REPEL_RADIUS) {
        const f = (REPEL_FORCE * (1 - dist / REPEL_RADIUS)) / dist;
        vx += dx * f * dt;
        vy += dy * f * dt;
      }
    }

    // Home recovery — subtle while on-screen, more decisive once a bee has been
    // pushed into the bleed area outside the visible viewport.
    const outside =
      Math.max(visibleLeft - x, 0) +
      Math.max(x - visibleRight, 0) +
      Math.max(visibleTop - y, 0) +
      Math.max(y - visibleBottom, 0);
    const offscreenRatio = inset > 0 ? Math.min(1, outside / inset) : 0;
    const homePull = HOME_PULL + OFFSCREEN_HOME_PULL * offscreenRatio;
    vx += (homeX - x) * homePull * dt;
    vy += (homeY - y) * homePull * dt;

    // Soft edge containment — inward acceleration ramping up near each edge.
    if (x < MARGIN) vx += EDGE_FORCE * (1 - x / MARGIN) * dt;
    else if (x > w - MARGIN) vx -= EDGE_FORCE * (1 - (w - x) / MARGIN) * dt;
    if (y < MARGIN) vy += EDGE_FORCE * (1 - y / MARGIN) * dt;
    else if (y > h - MARGIN) vy -= EDGE_FORCE * (1 - (h - y) / MARGIN) * dt;

    vx *= DAMP;
    vy *= DAMP;
    x += vx * dt;
    y += vy * dt;

    // Hard clamp as a final safety so bees never leave the frame; bounce the
    // outward velocity so a clamped bee turns back in rather than sticking.
    if (x < 0) {
      x = 0;
      if (vx < 0) vx = -vx * 0.5;
    } else if (x > w) {
      x = w;
      if (vx > 0) vx = -vx * 0.5;
    }
    if (y < 0) {
      y = 0;
      if (vy < 0) vy = -vy * 0.5;
    } else if (y > h) {
      y = h;
      if (vy > 0) vy = -vy * 0.5;
    }

    return { x, y, vx, vy, heading, seed, homeX, homeY };
  });
}
