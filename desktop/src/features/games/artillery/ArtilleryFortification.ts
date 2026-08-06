import Phaser from "phaser";

import type { ArtillerySide } from "@/features/games/artillery/referee";

type FortBlock = {
  baseAngle: number;
  baseX: number;
  baseY: number;
  object: Phaser.GameObjects.Rectangle;
};

const FORT_LAYOUT = {
  red: { backX: 91, color: 0xa94949, frontX: 224, groundY: 407 },
  blue: { backX: 869, color: 0x287da1, frontX: 726, groundY: 387 },
} as const;

/** A deterministic destructible fort whose state is derived from agent HP. */
export class ArtilleryFortification {
  private readonly blocks: FortBlock[] = [];
  private readonly flagObjects: Array<
    Phaser.GameObjects.Rectangle | Phaser.GameObjects.Triangle
  > = [];
  private integrity = 100;

  constructor(
    private readonly scene: Phaser.Scene,
    private readonly side: ArtillerySide,
  ) {
    this.build();
  }

  reset() {
    this.integrity = 100;
    for (const block of this.blocks) {
      block.object
        .setPosition(block.baseX, block.baseY)
        .setAngle(block.baseAngle)
        .setAlpha(1)
        .setVisible(true)
        .setFillStyle(FORT_LAYOUT[this.side].color, 1);
    }
    for (const object of this.flagObjects) object.setVisible(true);
  }

  setIntegrity(nextIntegrity: number, animate: boolean) {
    const clamped = Phaser.Math.Clamp(nextIntegrity, 0, 100);
    const previousVisible = this.visibleBlockCount(this.integrity);
    const nextVisible = this.visibleBlockCount(clamped);
    const destroyedCount = this.blocks.length - nextVisible;
    const previousDestroyedCount = this.blocks.length - previousVisible;
    this.integrity = clamped;
    for (const object of this.flagObjects) object.setVisible(clamped > 0);

    for (const [index, block] of this.blocks.entries()) {
      const shouldRemain = index >= destroyedCount;
      const newlyDestroyed =
        index >= previousDestroyedCount && index < destroyedCount;
      if (shouldRemain) {
        block.object
          .setVisible(true)
          .setAlpha(1)
          .setFillStyle(this.damageColor(clamped), 1);
      } else if (newlyDestroyed && animate) {
        this.crumbleBlock(block, index);
      } else {
        block.object.setVisible(false);
      }
    }
  }

  private visibleBlockCount(integrity: number) {
    if (integrity <= 0) return 0;
    return Math.ceil((integrity / 100) * this.blocks.length);
  }

  private damageColor(integrity: number) {
    if (integrity <= 30) return 0x744a3c;
    if (integrity <= 60) return 0x9a6148;
    return FORT_LAYOUT[this.side].color;
  }

  private crumbleBlock(block: FortBlock, index: number) {
    const direction = this.side === "red" ? 1 : -1;
    block.object.setVisible(true);
    this.scene.tweens.add({
      targets: block.object,
      x: block.baseX + direction * (16 + (index % 3) * 7),
      y: block.baseY + 28 + (index % 2) * 8,
      angle: direction * (32 + index * 7),
      alpha: 0,
      duration: 460 + index * 35,
      ease: "Quad.easeIn",
      onComplete: () => block.object.setVisible(false),
    });
  }

  private build() {
    const layout = FORT_LAYOUT[this.side];
    const direction = this.side === "red" ? 1 : -1;
    const pieces = [
      [layout.frontX, layout.groundY - 54, 19, 18],
      [layout.frontX, layout.groundY - 35, 19, 18],
      [layout.frontX, layout.groundY - 16, 23, 20],
      [layout.backX, layout.groundY - 65, 30, 18],
      [layout.backX, layout.groundY - 45, 27, 20],
      [layout.backX, layout.groundY - 23, 27, 22],
      [(layout.frontX + layout.backX) / 2, layout.groundY - 4, 112, 12],
    ] as const;

    for (const [x, y, width, height] of pieces) {
      const object = this.scene.add
        .rectangle(x, y, width, height, layout.color, 1)
        .setStrokeStyle(2, 0xf1d4a5, 0.28)
        .setDepth(4);
      this.blocks.push({ baseAngle: 0, baseX: x, baseY: y, object });
    }

    const pole = this.scene.add
      .rectangle(layout.backX, layout.groundY - 84, 4, 24, 0xc9d5dc, 0.9)
      .setDepth(4);
    const flag = this.scene.add
      .triangle(
        layout.backX + direction * 10,
        layout.groundY - 94,
        0,
        0,
        direction * 22,
        7,
        0,
        14,
        layout.color,
        1,
      )
      .setDepth(4);
    this.flagObjects.push(pole, flag);
  }
}
