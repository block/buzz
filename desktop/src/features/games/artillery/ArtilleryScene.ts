import Phaser from "phaser";

import {
  pointAtTime,
  type ArtilleryAnimationManifest,
} from "@/features/games/artillery/manifest";
import { ArtilleryFortification } from "@/features/games/artillery/ArtilleryFortification";
import {
  type ArtilleryRavineCinematicHandle,
  type ArtilleryRavineLoser,
  playArtilleryRavineCinematic,
} from "@/features/games/artillery/ArtilleryRavineCinematic";
import type {
  ArtilleryMatch,
  ArtillerySide,
} from "@/features/games/artillery/referee";

export type ArtilleryAnimationPhase =
  | "loading"
  | "ready"
  | "firing"
  | "impact"
  | "complete";

type ArtillerySceneCallbacks = {
  onPhaseChange: (phase: ArtilleryAnimationPhase) => void;
  onRunChange: (run: number) => void;
  onSoundCue: (cue: "launch" | "impact" | "victory") => void;
  onWhistleChange: (active: boolean, durationMs?: number) => void;
  onStructureChange: (side: ArtillerySide, integrity: number) => void;
  onTurnChange: (
    turnIndex: number,
    manifest: ArtilleryAnimationManifest,
  ) => void;
  onMatchComplete: (
    winner: ArtillerySide | "draw",
    reason: "elimination" | "forfeit",
  ) => void;
};

const WORLD_WIDTH = 960;
const WORLD_HEIGHT = 540;
const SKY_COLORS = [0x07142d, 0x0b2140, 0x103253, 0x174364, 0x205570];

export class ArtilleryScene extends Phaser.Scene {
  private match: ArtilleryMatch;
  private matchComplete: boolean;
  private readonly callbacks: ArtillerySceneCallbacks;
  private readonly reducedMotion: boolean;
  private manifest?: ArtilleryAnimationManifest;
  private projectile?: Phaser.GameObjects.Arc;
  private projectileGlow?: Phaser.GameObjects.Arc;
  private trail?: Phaser.GameObjects.Graphics;
  private redHealthFill?: Phaser.GameObjects.Graphics;
  private blueHealthFill?: Phaser.GameObjects.Graphics;
  private redHealthText?: Phaser.GameObjects.Text;
  private blueHealthText?: Phaser.GameObjects.Text;
  private turnText?: Phaser.GameObjects.Text;
  private inputText?: Phaser.GameObjects.Text;
  private health: Record<ArtillerySide, number> = { red: 100, blue: 100 };
  private turnIndex = -1;
  private run = 0;
  private shotTween?: Phaser.Tweens.Tween;
  private advanceTimer?: Phaser.Time.TimerEvent;
  private impactObjects: Phaser.GameObjects.GameObject[] = [];
  private awaitingNextTurn = false;
  private finalNotified = false;
  private forts?: Record<ArtillerySide, ArtilleryFortification>;
  private ravineCinematic?: ArtilleryRavineCinematicHandle;

  constructor(
    match: ArtilleryMatch,
    callbacks: ArtillerySceneCallbacks,
    reducedMotion: boolean,
    matchComplete = true,
  ) {
    super({ key: "buzz-artillery-match" });
    this.match = match;
    this.callbacks = callbacks;
    this.reducedMotion = reducedMotion;
    this.matchComplete = matchComplete;
    this.health = { ...match.initialHealth };
  }

  create() {
    this.drawSky();
    this.drawTerrain();
    this.drawArenaDetails();
    this.forts = {
      blue: new ArtilleryFortification(this, "blue"),
      red: new ArtilleryFortification(this, "red"),
    };
    this.drawAgent(157, 377, this.match.agents.red.name, 0xff6b6b, false);
    this.drawAgent(793, 357, this.match.agents.blue.name, 0x55c9ff, true);
    this.drawHud();

    this.trail = this.add.graphics().setDepth(6);
    this.projectileGlow = this.add
      .circle(166, 364, 15, 0xffc857, 0.18)
      .setDepth(7)
      .setVisible(false);
    this.projectile = this.add
      .circle(166, 364, 6, 0xffe8a3)
      .setStrokeStyle(2, 0xffffff, 0.9)
      .setDepth(8)
      .setVisible(false);

    this.callbacks.onPhaseChange("ready");
    this.time.delayedCall(420, () => this.replayMatch());
  }

  updateMatch(match: ArtilleryMatch, matchComplete: boolean) {
    this.match = match;
    this.matchComplete = matchComplete;
    if (!this.awaitingNextTurn) return;

    const nextTurnIndex = this.turnIndex + 1;
    if (this.match.turns[nextTurnIndex]) {
      this.awaitingNextTurn = false;
      this.playTurn(nextTurnIndex);
    } else if (this.matchComplete) {
      this.notifyMatchComplete();
    }
  }

  replay() {
    this.replayMatch();
  }

  replayMatch() {
    if (!this.projectile || !this.projectileGlow || !this.trail) return;

    this.shotTween?.stop();
    this.callbacks.onWhistleChange(false);
    this.tweens.killAll();
    this.advanceTimer?.destroy();
    this.clearImpact();
    this.trail.clear();
    this.projectile.setVisible(false);
    this.projectileGlow.setVisible(false);
    this.health = { ...this.match.initialHealth };
    this.forts?.red.reset();
    this.forts?.blue.reset();
    this.callbacks.onStructureChange("red", 100);
    this.callbacks.onStructureChange("blue", 100);
    this.turnIndex = -1;
    this.awaitingNextTurn = false;
    this.finalNotified = false;
    this.drawHealthBars();
    this.run += 1;
    this.callbacks.onRunChange(this.run);
    this.playTurn(0);
  }

  pauseMatch() {
    this.callbacks.onWhistleChange(false);
    this.scene.pause();
  }

  resumeMatch() {
    this.scene.resume();
    if (this.manifest && this.shotTween?.isPlaying()) {
      this.callbacks.onWhistleChange(
        true,
        this.manifest.durationMs * (1 - this.shotTween.progress),
      );
    }
  }

  playLoserRavine(loser: ArtilleryRavineLoser) {
    if (this.ravineCinematic) return this.ravineCinematic.finished;
    this.shotTween?.stop();
    this.callbacks.onWhistleChange(false);
    this.advanceTimer?.destroy();
    this.ravineCinematic = playArtilleryRavineCinematic(
      this,
      loser,
      this.reducedMotion,
    );
    return this.ravineCinematic.finished.finally(() => {
      this.ravineCinematic = undefined;
    });
  }

  skipLoserRavine() {
    this.ravineCinematic?.skip();
  }

  forfeit(loser: ArtillerySide) {
    this.shotTween?.stop();
    this.callbacks.onWhistleChange(false);
    this.tweens.killAll();
    this.advanceTimer?.destroy();
    this.projectile?.setVisible(false);
    this.projectileGlow?.setVisible(false);
    this.callbacks.onPhaseChange("complete");
    this.callbacks.onSoundCue("victory");
    this.callbacks.onMatchComplete(loser === "red" ? "blue" : "red", "forfeit");
  }

  private playTurn(index: number) {
    const turn = this.match.turns[index];
    if (!this.projectile || !this.projectileGlow || !this.trail) return;
    if (!turn) {
      if (this.matchComplete) this.notifyMatchComplete();
      else this.waitForNextTurn();
      return;
    }

    this.awaitingNextTurn = false;
    this.turnIndex = index;
    this.manifest = turn.manifest;
    const start = this.manifest.trajectory[0] ?? { x: 0, y: 0 };
    this.clearImpact();
    this.trail.clear();
    this.updateHud();
    this.projectile.setVisible(true).setPosition(start.x, start.y).setAlpha(1);
    this.projectileGlow
      .setVisible(true)
      .setPosition(start.x, start.y)
      .setAlpha(1);
    this.callbacks.onTurnChange(index, this.manifest);
    this.callbacks.onPhaseChange("firing");
    this.callbacks.onSoundCue("launch");

    if (this.reducedMotion) {
      const endpoint = pointAtTime(this.manifest, this.manifest.durationMs);
      this.projectile.setPosition(endpoint.x, endpoint.y);
      this.projectileGlow.setPosition(endpoint.x, endpoint.y);
      this.showImpact(false);
      return;
    }

    this.callbacks.onWhistleChange(true, this.manifest.durationMs);
    const tweenState = { elapsed: 0 };
    this.shotTween = this.tweens.add({
      targets: tweenState,
      elapsed: this.manifest.durationMs,
      duration: this.manifest.durationMs,
      ease: "Linear",
      onUpdate: () => this.renderProjectile(tweenState.elapsed),
      onComplete: () => this.showImpact(true),
    });
  }

  private renderProjectile(elapsedMs: number) {
    if (
      !this.projectile ||
      !this.projectileGlow ||
      !this.trail ||
      !this.manifest
    )
      return;

    const point = pointAtTime(this.manifest, elapsedMs);
    const next = pointAtTime(this.manifest, elapsedMs + 16);
    this.projectile.setPosition(point.x, point.y);
    this.projectileGlow.setPosition(point.x, point.y);
    this.projectile.setRotation(Math.atan2(next.y - point.y, next.x - point.x));

    this.trail.clear();
    const trailStart = Math.max(0, elapsedMs - 420);
    for (let time = trailStart; time <= elapsedMs; time += 42) {
      const trailPoint = pointAtTime(this.manifest, time);
      const age = (time - trailStart) / Math.max(1, elapsedMs - trailStart);
      this.trail.fillStyle(0xffd37a, age * 0.52);
      this.trail.fillCircle(trailPoint.x, trailPoint.y, 1.5 + age * 2.2);
    }
  }

  private showImpact(animate: boolean) {
    if (!this.projectile || !this.projectileGlow || !this.manifest) return;

    this.callbacks.onWhistleChange(false);
    this.callbacks.onPhaseChange("impact");
    this.callbacks.onSoundCue("impact");
    this.projectile.setVisible(false);
    this.projectileGlow.setVisible(false);
    this.health[this.manifest.damage.target] = this.manifest.damage.after;
    this.forts?.[this.manifest.damage.target].setIntegrity(
      this.manifest.damage.after,
      animate,
    );
    this.callbacks.onStructureChange(
      this.manifest.damage.target,
      this.manifest.damage.after,
    );
    this.drawHealthBars();

    const { x, y, radius } = this.manifest.impact;
    const flash = this.add
      .circle(x, y, radius * 0.35, 0xffffff, 0.95)
      .setDepth(12);
    const blast = this.add
      .circle(x, y, radius, 0xffa726, 0.88)
      .setStrokeStyle(5, 0xffe28a, 0.9)
      .setDepth(11);
    const shockwave = this.add
      .circle(x, y, radius * 0.55, 0xffd166, 0.05)
      .setStrokeStyle(4, 0xffd166, 0.82)
      .setDepth(10);
    this.impactObjects.push(flash, blast, shockwave);

    const debrisColors = [0xffc857, 0xff8c42, 0xe85d3f, 0xd7e4ee];
    for (let index = 0; index < 18; index += 1) {
      const angle = (Math.PI * 2 * index) / 18;
      const distance = 34 + (index % 4) * 9;
      const debris = this.add
        .rectangle(
          x,
          y,
          4 + (index % 3),
          4 + ((index + 1) % 3),
          debrisColors[index % debrisColors.length],
        )
        .setRotation(angle)
        .setDepth(13);
      this.impactObjects.push(debris);
      if (animate) {
        this.tweens.add({
          targets: debris,
          x: x + Math.cos(angle) * distance,
          y: y + Math.sin(angle) * distance + 22,
          angle: Phaser.Math.RadToDeg(angle) + 140,
          alpha: 0,
          duration: 620 + (index % 4) * 65,
          ease: "Quad.easeOut",
        });
      }
    }

    if (!animate) {
      flash.setAlpha(0.25);
      blast.setScale(1.15).setAlpha(0.76);
      shockwave.setScale(1.5);
      this.finishTurn();
      return;
    }

    this.cameras.main.shake(220, 0.006);
    this.tweens.add({
      targets: flash,
      scale: 2.4,
      alpha: 0,
      duration: 260,
      ease: "Quad.easeOut",
    });
    this.tweens.add({
      targets: blast,
      scale: 1.55,
      alpha: 0.2,
      duration: 520,
      ease: "Cubic.easeOut",
    });
    this.tweens.add({
      targets: shockwave,
      scale: 2.3,
      alpha: 0,
      duration: 670,
      ease: "Cubic.easeOut",
      onComplete: () => this.finishTurn(),
    });
  }

  private finishTurn() {
    this.callbacks.onPhaseChange("complete");
    const nextTurnIndex = this.turnIndex + 1;
    if (!this.match.turns[nextTurnIndex]) {
      if (this.matchComplete) this.notifyMatchComplete();
      else this.waitForNextTurn();
      return;
    }
    this.advanceTimer = this.time.delayedCall(
      this.reducedMotion ? 20 : 520,
      () => this.playTurn(nextTurnIndex),
    );
  }

  private waitForNextTurn() {
    this.awaitingNextTurn = true;
    this.callbacks.onPhaseChange("ready");
    this.turnText?.setText(
      this.turnIndex < 0 ? "MATCH LIVE" : "WAITING FOR NEXT MOVE",
    );
    this.inputText?.setText("AGENTS ARE CHOOSING THEIR SHOT");
  }

  private notifyMatchComplete() {
    if (this.finalNotified) return;
    this.finalNotified = true;
    this.awaitingNextTurn = false;
    this.callbacks.onPhaseChange("complete");
    this.callbacks.onSoundCue("victory");
    this.callbacks.onMatchComplete(this.match.winner, "elimination");
  }

  private clearImpact() {
    for (const object of this.impactObjects) object.destroy();
    this.impactObjects = [];
  }

  private drawSky() {
    for (let index = 0; index < SKY_COLORS.length; index += 1) {
      this.add
        .rectangle(
          WORLD_WIDTH / 2,
          (WORLD_HEIGHT / SKY_COLORS.length) * index +
            WORLD_HEIGHT / SKY_COLORS.length / 2,
          WORLD_WIDTH,
          WORLD_HEIGHT / SKY_COLORS.length + 2,
          SKY_COLORS[index],
        )
        .setDepth(0);
    }

    const stars = [
      [74, 66, 2],
      [142, 112, 1],
      [231, 52, 1],
      [326, 92, 2],
      [431, 56, 1],
      [536, 108, 1],
      [628, 48, 2],
      [735, 91, 1],
      [846, 58, 1],
      [902, 126, 2],
      [496, 154, 1],
      [278, 151, 1],
    ];
    for (const [x, y, size] of stars) {
      this.add.circle(x, y, size, 0xe7f8ff, 0.75).setDepth(1);
    }

    this.add.circle(820, 92, 43, 0xffdf9c, 0.12).setDepth(1);
    this.add.circle(820, 92, 29, 0xffe7b0, 0.92).setDepth(2);
    this.add.circle(808, 83, 6, 0xd9c28d, 0.28).setDepth(3);
    this.add.circle(832, 100, 4, 0xd9c28d, 0.25).setDepth(3);
  }

  private drawTerrain() {
    const far = this.add.graphics().setDepth(2);
    far.fillStyle(0x173c4e, 1);
    far.beginPath();
    far.moveTo(0, 330);
    far.lineTo(110, 260);
    far.lineTo(216, 319);
    far.lineTo(343, 225);
    far.lineTo(474, 313);
    far.lineTo(610, 242);
    far.lineTo(759, 302);
    far.lineTo(884, 235);
    far.lineTo(960, 281);
    far.lineTo(960, 540);
    far.lineTo(0, 540);
    far.closePath();
    far.fillPath();

    const terrain = this.add.graphics().setDepth(3);
    terrain.fillStyle(0x142f36, 1);
    terrain.lineStyle(4, 0x5c9c72, 1);
    terrain.beginPath();
    terrain.moveTo(0, 418);
    terrain.lineTo(74, 391);
    terrain.lineTo(142, 387);
    terrain.lineTo(205, 405);
    terrain.lineTo(285, 423);
    terrain.lineTo(381, 431);
    terrain.lineTo(475, 420);
    terrain.lineTo(563, 397);
    terrain.lineTo(649, 378);
    terrain.lineTo(730, 371);
    terrain.lineTo(819, 380);
    terrain.lineTo(896, 405);
    terrain.lineTo(960, 414);
    terrain.lineTo(960, 540);
    terrain.lineTo(0, 540);
    terrain.closePath();
    terrain.fillPath();
    terrain.strokePath();
  }

  private drawArenaDetails() {
    const city = this.add.graphics().setDepth(2);
    const buildings = [
      [27, 326, 38, 91],
      [70, 347, 31, 68],
      [110, 318, 44, 97],
      [853, 327, 38, 79],
      [897, 300, 42, 107],
      [940, 340, 29, 70],
    ];
    for (const [x, y, width, height] of buildings) {
      city.fillStyle(0x0c2636, 0.95);
      city.fillRect(x, y, width, height);
      city.fillStyle(0xffd166, 0.42);
      for (let row = y + 12; row < y + height - 5; row += 16) {
        city.fillRect(x + 8, row, 5, 7);
        city.fillRect(x + width - 13, row, 5, 7);
      }
    }

    const windLine = this.add.graphics().setDepth(2);
    windLine.lineStyle(2, 0x8be0ef, 0.18);
    windLine.beginPath();
    windLine.moveTo(390, 178);
    windLine.lineTo(444, 164);
    windLine.lineTo(510, 183);
    windLine.lineTo(582, 167);
    windLine.strokePath();
  }

  private drawAgent(
    x: number,
    y: number,
    label: string,
    color: number,
    facesLeft: boolean,
  ) {
    const shadow = this.add
      .ellipse(x, y + 24, 72, 15, 0x000000, 0.34)
      .setDepth(4);
    const body = this.add.graphics().setDepth(5);
    body.fillStyle(color, 1);
    body.fillRoundedRect(x - 30, y - 13, 60, 34, 9);
    body.fillStyle(0x12212d, 1);
    body.fillRoundedRect(x - 18, y - 30, 36, 24, 8);
    body.fillStyle(0xbdf5ff, 0.9);
    body.fillCircle(x - 7, y - 18, 3);
    body.fillCircle(x + 7, y - 18, 3);
    body.fillStyle(0x1a252f, 1);
    body.fillCircle(x - 20, y + 21, 11);
    body.fillCircle(x + 20, y + 21, 11);

    const barrel = this.add.rectangle(
      x + (facesLeft ? -31 : 31),
      y - 10,
      42,
      7,
      color,
    );
    barrel.setOrigin(facesLeft ? 1 : 0, 0.5);
    barrel.setRotation(facesLeft ? -0.45 : -0.73).setDepth(5);

    this.add
      .text(x, y + 42, `AGENT ${label}`, {
        color: Phaser.Display.Color.IntegerToColor(color).rgba,
        fontFamily: "Inter, sans-serif",
        fontSize: "13px",
        fontStyle: "bold",
        letterSpacing: 1.4,
      })
      .setOrigin(0.5)
      .setDepth(5);

    shadow.setAlpha(0.72);
  }

  private drawHud() {
    this.add.rectangle(480, 31, 920, 45, 0x07111f, 0.72).setDepth(20);
    this.redHealthText = this.add
      .text(42, 20, "", {
        color: "#ff8585",
        fontFamily: "Inter, sans-serif",
        fontSize: "15px",
        fontStyle: "bold",
      })
      .setDepth(21);
    this.turnText = this.add
      .text(480, 20, "MATCH READY", {
        color: "#d7edf5",
        fontFamily: "Inter, sans-serif",
        fontSize: "14px",
        fontStyle: "bold",
      })
      .setOrigin(0.5, 0)
      .setDepth(21);
    this.blueHealthText = this.add
      .text(918, 20, "", {
        color: "#71d3ff",
        fontFamily: "Inter, sans-serif",
        fontSize: "15px",
        fontStyle: "bold",
      })
      .setOrigin(1, 0)
      .setDepth(21);

    const panel = this.add.graphics().setDepth(20);
    panel.fillStyle(0x07111f, 0.76);
    panel.fillRoundedRect(335, 478, 290, 42, 13);
    this.inputText = this.add
      .text(480, 490, "WAITING FOR REFEREE", {
        color: "#d7edf5",
        fontFamily: "JetBrains Mono, monospace",
        fontSize: "13px",
      })
      .setOrigin(0.5, 0)
      .setDepth(21);

    this.add.rectangle(104, 54, 124, 5, 0x1b3442, 0.9).setDepth(21);
    this.add.rectangle(856, 54, 124, 5, 0x1b3442, 0.9).setDepth(21);
    this.drawHealthBars();
  }

  private drawHealthBars() {
    this.redHealthText?.setText(
      `${this.match.agents.red.name.toUpperCase()}  ${this.health.red} HP`,
    );
    this.blueHealthText?.setText(
      `${this.health.blue} HP  ${this.match.agents.blue.name.toUpperCase()}`,
    );
    this.redHealthFill?.destroy();
    this.blueHealthFill?.destroy();
    const width = 124;
    this.redHealthFill = this.add.graphics().setDepth(22);
    this.redHealthFill.fillStyle(this.health.red > 55 ? 0xff6b6b : 0xffb347, 1);
    this.redHealthFill.fillRoundedRect(
      42,
      51.5,
      width * (this.health.red / 100),
      5,
      2,
    );
    this.blueHealthFill = this.add.graphics().setDepth(22);
    this.blueHealthFill.fillStyle(
      this.health.blue > 55 ? 0x55c9ff : 0xffb347,
      1,
    );
    this.blueHealthFill.fillRoundedRect(
      794,
      51.5,
      width * (this.health.blue / 100),
      5,
      2,
    );
  }

  private updateHud() {
    if (!this.manifest) return;
    const shooter = this.manifest.shooterName ?? this.manifest.shooter;
    const fallback = this.manifest.resolution?.includes("fallback")
      ? " • SAFE FALLBACK"
      : "";
    this.turnText?.setText(
      `TURN ${String(this.manifest.turn ?? this.turnIndex + 1).padStart(2, "0")}  •  ${shooter.toUpperCase()} FIRING${fallback}`,
    );
    const windDirection = this.manifest.wind < 0 ? "←" : "→";
    this.inputText?.setText(
      `${this.manifest.angle}°  •  POWER ${this.manifest.power}  •  WIND ${windDirection} ${Math.abs(this.manifest.wind)}`,
    );
  }
}

export const ARTILLERY_WORLD_SIZE = {
  width: WORLD_WIDTH,
  height: WORLD_HEIGHT,
} as const;
