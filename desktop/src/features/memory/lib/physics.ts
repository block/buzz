import type { SimNode, SimLink, GraphPhysicsConfig } from "./types";

export const DEFAULT_PHYSICS_CONFIG: GraphPhysicsConfig = {
  chargeRepulsion: 380,
  linkDistance: 110,
  centerGravity: 0.035,
  collisionRadius: 28,
  velocityDecay: 0.86,
};

export class BrainPhysicsSimulation {
  private nodes: SimNode[] = [];
  private links: SimLink[] = [];
  private config: GraphPhysicsConfig;
  private width: number;
  private height: number;

  constructor(
    width = 1200,
    height = 800,
    config: Partial<GraphPhysicsConfig> = {},
  ) {
    this.width = width;
    this.height = height;
    this.config = { ...DEFAULT_PHYSICS_CONFIG, ...config };
  }

  public setDimensions(width: number, height: number): void {
    this.width = Math.max(width, 100);
    this.height = Math.max(height, 100);
  }

  public setConfig(config: Partial<GraphPhysicsConfig>): void {
    this.config = { ...this.config, ...config };
  }

  public setGraph(nodes: SimNode[], links: SimLink[]): void {
    this.nodes = nodes;
    this.links = links;
  }

  public tick(): void {
    const { chargeRepulsion, linkDistance, centerGravity, collisionRadius, velocityDecay } =
      this.config;
    const cx = this.width / 2;
    const cy = this.height / 2;
    const count = this.nodes.length;
    if (count === 0) return;

    // 1. Repulsion between all pairs of nodes
    for (let i = 0; i < count; i++) {
      const n1 = this.nodes[i];
      for (let j = i + 1; j < count; j++) {
        const n2 = this.nodes[j];
        let dx = n2.x - n1.x;
        let dy = n2.y - n1.y;
        let dist = Math.sqrt(dx * dx + dy * dy);
        if (dist === 0) {
          dx = (Math.random() - 0.5) * 2;
          dy = (Math.random() - 0.5) * 2;
          dist = Math.sqrt(dx * dx + dy * dy);
        }

        // Repulsive force proportional to charge / dist^2
        const force = chargeRepulsion / (dist * dist + 100);
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;

        if (n1.fx == null) {
          n1.vx -= fx;
          n1.vy -= fy;
        }
        if (n2.fx == null) {
          n2.vx += fx;
          n2.vy += fy;
        }

        // Collision separation
        const minGap = collisionRadius + (n1.radius + n2.radius) * 0.5;
        if (dist < minGap) {
          const overlap = minGap - dist;
          const pushX = (dx / dist) * overlap * 0.5;
          const pushY = (dy / dist) * overlap * 0.5;
          if (n1.fx == null) {
            n1.x -= pushX * 0.2;
            n1.y -= pushY * 0.2;
          }
          if (n2.fx == null) {
            n2.x += pushX * 0.2;
            n2.y += pushY * 0.2;
          }
        }
      }
    }

    // 2. Link Spring Force
    for (let i = 0; i < this.links.length; i++) {
      const link = this.links[i];
      const source = link.sourceNode;
      const target = link.targetNode;
      if (!source || !target) continue;

      let dx = target.x - source.x;
      let dy = target.y - source.y;
      let dist = Math.sqrt(dx * dx + dy * dy);
      if (dist === 0) {
        dx = 1;
        dist = 1;
      }

      // Spring displacement
      const delta = dist - linkDistance;
      const springK = 0.05 * (link.is_active ? 1.0 : 0.6);
      const forceX = (dx / dist) * delta * springK;
      const forceY = (dy / dist) * delta * springK;

      if (source.fx == null) {
        source.vx += forceX;
        source.vy += forceY;
      }
      if (target.fx == null) {
        target.vx -= forceX;
        target.vy -= forceY;
      }
    }

    // 3. Central Gravity & Euler Update
    for (let i = 0; i < count; i++) {
      const node = this.nodes[i];
      if (node.fx != null && node.fy != null) {
        node.x = node.fx;
        node.y = node.fy;
        node.vx = 0;
        node.vy = 0;
        continue;
      }

      // Gentle pull toward canvas center
      node.vx += (cx - node.x) * centerGravity;
      node.vy += (cy - node.y) * centerGravity;

      // Velocity decay / friction
      node.vx *= velocityDecay;
      node.vy *= velocityDecay;

      // Integrate position
      node.x += node.vx;
      node.y += node.vy;
    }
  }
}
