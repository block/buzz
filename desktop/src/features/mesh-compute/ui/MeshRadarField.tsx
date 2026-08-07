import * as React from "react";
import { useReducedMotion } from "motion/react";

import type { MeshDeviceState } from "@/shared/api/tauriMesh";
import { cn } from "@/shared/lib/cn";

/**
 * One node on the ring, from either source.
 *
 * Live gossip peers carry an RTT and are reachable *now*. Relay status notes
 * carry no RTT and are only *last known* (valid 120s), so they land mid-band.
 * The caller says which it passed via `live`, and the caption names it — the
 * field never implies reachability it cannot show.
 */
export type MeshFieldNode = {
  id: string;
  state: MeshDeviceState;
  capacityGb: number | null;
  rttMs: number | null;
};

/**
 * Canvas-2D radar field for the mesh.
 *
 * Visual language borrowed from mesh-llm's own console (self at centre, radius
 * from latency, size from VRAM, per-node breathing seeded by id) but drawn in
 * 2D rather than porting its ~2,200 lines of WebGL: a 288px popover does not
 * justify shader plumbing, and this has to stay cheap enough to run beside a
 * chat client.
 *
 * ## What is real here
 *
 * Every solid node is a peer this machine's runtime is **connected to right
 * now** — live gossip, not a relay note that outlives its writer. Edge geometry
 * is real adjacency and edge opacity is real RTT.
 *
 * Nothing travels along an edge. mesh-llm's counters are node-local and carry
 * no per-edge attribution, so a packet gliding down a spoke would be invented.
 * Motion here is confined to what we can defend: a node breathes, and the
 * centre pulses when this machine has work in flight.
 *
 * Ghosts are community members with no published status note. They are drawn as
 * a dashed outer band with **no size and no capacity**, because a member who
 * never started a node has disclosed no hardware. They are a count, never GB.
 */

/** Stable 0..1 from a node id, so each node breathes on its own phase. */
function hashUnit(value: string): number {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return ((hash >>> 0) % 10000) / 10000;
}

type Palette = {
  self: string;
  serving: string;
  consuming: string;
  standby: string;
  edge: string;
  ghost: string;
};

const LIGHT: Palette = {
  self: "16, 185, 129",
  serving: "16, 185, 129",
  consuming: "14, 165, 233",
  standby: "148, 163, 184",
  edge: "100, 116, 139",
  ghost: "148, 163, 184",
};

const DARK: Palette = {
  self: "52, 211, 153",
  serving: "52, 211, 153",
  consuming: "56, 189, 248",
  standby: "148, 163, 184",
  edge: "148, 163, 184",
  ghost: "148, 163, 184",
};

function peerColor(state: MeshDeviceState, palette: Palette): string {
  if (state === "serving") return palette.serving;
  if (state === "consuming") return palette.consuming;
  return palette.standby;
}

/** Radius band by RTT: closer nodes sit nearer the centre. */
function radiusFor(rttMs: number | null, min: number, max: number): number {
  if (rttMs === null) return (min + max) / 2;
  // 0..120ms maps across the band; beyond that everything pins to the rim.
  const t = Math.min(1, Math.max(0, rttMs / 120));
  return min + (max - min) * t;
}

/** Node radius from shared capacity, with a floor so a client is still visible. */
function sizeFor(capacityGb: number | null): number {
  if (capacityGb === null || capacityGb <= 0) return 3.2;
  return Math.min(7, 3.4 + Math.sqrt(capacityGb) * 0.45);
}

export function MeshRadarField({
  nodes,
  selfCapacityGb,
  selfParticipating,
  ghostCount,
  busyNow,
  className,
  height = 150,
}: {
  nodes: MeshFieldNode[];
  selfCapacityGb: number | null;
  /**
   * Whether this machine is in the mesh. When false the centre is drawn hollow:
   * we are looking at a pool we have not joined, and a solid centre would claim
   * membership we do not have.
   */
  selfParticipating: boolean;
  /** Community members with no node. Drawn as a dashed band, count only. */
  ghostCount: number;
  busyNow: boolean;
  className?: string;
  height?: number;
}) {
  const shouldReduceMotion = useReducedMotion();
  const canvasRef = React.useRef<HTMLCanvasElement | null>(null);
  const [isDark, setIsDark] = React.useState(false);

  // Palette follows the app theme; the canvas cannot inherit CSS variables.
  React.useEffect(() => {
    const read = () =>
      setIsDark(document.documentElement.classList.contains("dark"));
    read();
    const observer = new MutationObserver(read);
    observer.observe(document.documentElement, {
      attributeFilter: ["class"],
      attributes: true,
    });
    return () => observer.disconnect();
  }, []);

  // Snapshot the frame inputs so the draw loop is not re-created per render.
  const frame = React.useRef({
    nodes,
    selfCapacityGb,
    selfParticipating,
    ghostCount,
    busyNow,
    isDark,
  });
  frame.current = {
    nodes,
    selfCapacityGb,
    selfParticipating,
    ghostCount,
    busyNow,
    isDark,
  };

  React.useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const context = canvas.getContext("2d");
    if (!context) return;

    let raf = 0;
    let disposed = false;
    const started = performance.now();

    const draw = (now: number) => {
      if (disposed) return;
      const {
        nodes: fieldNodes,
        selfCapacityGb: selfGb,
        selfParticipating: joined,
        ghostCount: ghosts,
        busyNow: busy,
        isDark: dark,
      } = frame.current;
      const palette = dark ? DARK : LIGHT;

      const ratio = Math.min(2, globalThis.devicePixelRatio || 1);
      const width = canvas.clientWidth;
      const drawHeight = canvas.clientHeight;
      if (canvas.width !== Math.round(width * ratio)) {
        canvas.width = Math.round(width * ratio);
        canvas.height = Math.round(drawHeight * ratio);
      }
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
      context.clearRect(0, 0, width, drawHeight);

      const cx = width / 2;
      const cy = drawHeight / 2;
      // Reduced motion freezes the clock rather than removing the geometry, so
      // the field still reads the same — it simply stops breathing.
      const elapsed = shouldReduceMotion ? 0 : (now - started) / 1000;
      const rim = Math.min(cx, cy) - 12;
      const innerMin = rim * 0.42;
      const innerMax = rim * 0.82;

      // Faint range rings for depth.
      context.strokeStyle = `rgba(${palette.edge}, ${dark ? 0.1 : 0.09})`;
      context.lineWidth = 1;
      for (const scale of [0.42, 0.82]) {
        context.beginPath();
        context.arc(cx, cy, rim * scale, 0, Math.PI * 2);
        context.stroke();
      }

      // Ghost band: members with no node. Dashed, sizeless, no capacity implied.
      if (ghosts > 0) {
        context.save();
        context.setLineDash([2, 3]);
        context.strokeStyle = `rgba(${palette.ghost}, ${dark ? 0.22 : 0.2})`;
        context.beginPath();
        context.arc(cx, cy, rim, 0, Math.PI * 2);
        context.stroke();
        context.setLineDash([]);
        const shown = Math.min(ghosts, 14);
        for (let index = 0; index < shown; index += 1) {
          const angle = (index / shown) * Math.PI * 2 - Math.PI / 2 + 0.31;
          const x = cx + Math.cos(angle) * rim;
          const y = cy + Math.sin(angle) * rim;
          context.beginPath();
          context.arc(x, y, 1.8, 0, Math.PI * 2);
          context.fillStyle = `rgba(${palette.ghost}, ${dark ? 0.34 : 0.3})`;
          context.fill();
        }
        context.restore();
      }

      // Peers: angle spread evenly, radius from RTT, size from capacity.
      const count = fieldNodes.length;
      fieldNodes.forEach((peer, index) => {
        const spread = count === 0 ? 0 : (index / count) * Math.PI * 2;
        const angle = spread - Math.PI / 2 + hashUnit(peer.id) * 0.4;
        const radius = radiusFor(peer.rttMs, innerMin, innerMax);
        const x = cx + Math.cos(angle) * radius;
        const y = cy + Math.sin(angle) * radius;
        const rgb = peerColor(peer.state, palette);

        // A spoke asserts "this machine is connected to that node". True only
        // when we have joined; from the outside the nodes are real but our
        // adjacency to them is not, so the spokes are simply omitted.
        if (joined) {
          // Opacity carries RTT — closer reads stronger.
          const proximity =
            peer.rttMs === null ? 0.5 : 1 - Math.min(1, peer.rttMs / 120);
          context.strokeStyle = `rgba(${palette.edge}, ${0.12 + proximity * 0.22})`;
          context.lineWidth = 1;
          context.beginPath();
          context.moveTo(cx, cy);
          context.lineTo(x, y);
          context.stroke();
        }

        // Breathing, phase-seeded by id so the field shimmers rather than blinks.
        const phase = hashUnit(`${peer.id}:phase`) * Math.PI * 2;
        const breath =
          peer.state === "serving"
            ? 0.82 + Math.sin(elapsed * 1.7 + phase) * 0.18
            : 0.9;
        const size = sizeFor(peer.capacityGb) * breath;

        const glow = context.createRadialGradient(x, y, 0, x, y, size * 3.4);
        glow.addColorStop(0, `rgba(${rgb}, ${dark ? 0.36 : 0.3})`);
        glow.addColorStop(1, `rgba(${rgb}, 0)`);
        context.fillStyle = glow;
        context.beginPath();
        context.arc(x, y, size * 3.4, 0, Math.PI * 2);
        context.fill();

        context.fillStyle = `rgba(${rgb}, ${peer.state === "serving" ? 0.95 : 0.72})`;
        context.beginPath();
        context.arc(x, y, size, 0, Math.PI * 2);
        context.fill();
      });

      // Self, last so it sits above the spokes.
      const selfSize = sizeFor(selfGb) + 1.4;
      if (busy && !shouldReduceMotion) {
        // Expanding ring: this machine has work in flight. Not attributed to any
        // edge, because nothing tells us which peer it belongs to.
        const wave = (elapsed % 1.6) / 1.6;
        context.strokeStyle = `rgba(${palette.self}, ${0.34 * (1 - wave)})`;
        context.lineWidth = 1.5;
        context.beginPath();
        context.arc(cx, cy, selfSize + wave * 22, 0, Math.PI * 2);
        context.stroke();
      }
      const selfGlow = context.createRadialGradient(
        cx,
        cy,
        0,
        cx,
        cy,
        selfSize * 3.6,
      );
      selfGlow.addColorStop(
        0,
        joined
          ? `rgba(${palette.self}, ${dark ? 0.42 : 0.34})`
          : `rgba(${palette.standby}, ${dark ? 0.16 : 0.12})`,
      );
      selfGlow.addColorStop(
        1,
        `rgba(${joined ? palette.self : palette.standby}, 0)`,
      );
      context.fillStyle = selfGlow;
      context.beginPath();
      context.arc(cx, cy, selfSize * 3.6, 0, Math.PI * 2);
      context.fill();

      if (joined) {
        context.fillStyle = `rgba(${palette.self}, 0.98)`;
        context.beginPath();
        context.arc(cx, cy, selfSize, 0, Math.PI * 2);
        context.fill();
        context.strokeStyle = dark
          ? "rgba(255, 255, 255, 0.5)"
          : "rgba(255, 255, 255, 0.85)";
        context.lineWidth = 1.4;
        context.stroke();
      } else {
        // Hollow centre: this is a pool we are looking at, not one we are in.
        context.strokeStyle = `rgba(${palette.standby}, ${dark ? 0.7 : 0.6})`;
        context.lineWidth = 1.4;
        context.beginPath();
        context.arc(cx, cy, selfSize, 0, Math.PI * 2);
        context.stroke();
      }

      // A frozen field needs no frames; reduced motion draws once and stops.
      if (!shouldReduceMotion) {
        raf = requestAnimationFrame(draw);
      }
    };

    raf = requestAnimationFrame(draw);
    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
    };
  }, [shouldReduceMotion]);

  return (
    <canvas
      aria-hidden
      className={cn("w-full rounded-lg", className)}
      data-testid="mesh-radar-field"
      ref={canvasRef}
      style={{ height }}
    />
  );
}
