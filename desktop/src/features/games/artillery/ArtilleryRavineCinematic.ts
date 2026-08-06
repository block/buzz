import type Phaser from "phaser";
import { startArtilleryRavineYell } from "@/features/games/artillery/artilleryAudio";

export type ArtilleryRavineLoser = {
  avatarUrl: string | null;
  name: string;
};

export type ArtilleryRavineCinematicHandle = {
  finished: Promise<void>;
  skip: () => void;
};

const WORLD_WIDTH = 960;
const WORLD_HEIGHT = 540;

/** Plays the post-deletion ravine send-off over the existing artillery scene. */
export function playArtilleryRavineCinematic(
  scene: Phaser.Scene,
  loser: ArtilleryRavineLoser,
  reducedMotion: boolean,
): ArtilleryRavineCinematicHandle {
  let active = true;
  let resolveFinished = () => {};
  let avatarKey: string | null = null;
  let stopYell = () => {};
  const finished = new Promise<void>((resolve) => {
    resolveFinished = resolve;
  });

  const root = scene.add.container(0, 0).setDepth(1_000);
  const landscape = scene.add.graphics();
  landscape.fillStyle(0x07111f, 0.98);
  landscape.fillRect(0, 0, WORLD_WIDTH, WORLD_HEIGHT);
  landscape.fillStyle(0x172b49, 1);
  landscape.fillCircle(770, 90, 56);
  landscape.fillStyle(0x9bd9ff, 0.22);
  landscape.fillCircle(754, 77, 48);

  for (const [x, y, radius] of [
    [90, 76, 2],
    [168, 124, 1.5],
    [280, 68, 1.5],
    [580, 104, 2],
    [864, 145, 1.5],
  ] as const) {
    landscape.fillStyle(0xd8efff, 0.78);
    landscape.fillCircle(x, y, radius);
  }

  landscape.fillStyle(0x02050b, 1);
  fillPolygon(landscape, [
    [355, 190],
    [620, 165],
    [710, WORLD_HEIGHT],
    [280, WORLD_HEIGHT],
  ]);
  landscape.fillStyle(0x23364a, 1);
  fillPolygon(landscape, [
    [0, 210],
    [355, 190],
    [405, 258],
    [336, WORLD_HEIGHT],
    [0, WORLD_HEIGHT],
  ]);
  landscape.fillStyle(0x2f4b5f, 1);
  fillPolygon(landscape, [
    [620, 165],
    [WORLD_WIDTH, 205],
    [WORLD_WIDTH, WORLD_HEIGHT],
    [655, WORLD_HEIGHT],
    [570, 244],
  ]);
  landscape.fillStyle(0x52716d, 1);
  landscape.fillRect(0, 196, 350, 18);
  landscape.fillStyle(0x3a5661, 1);
  landscape.fillTriangle(397, 317, 478, 333, 404, 352);
  landscape.fillTriangle(570, 390, 648, 377, 633, 415);
  landscape.fillTriangle(405, 458, 480, 471, 420, 493);
  root.add(landscape);

  const heading = scene.add
    .text(WORLD_WIDTH / 2, 58, `FAREWELL, ${loser.name.toUpperCase()}`, {
      color: "#f8fafc",
      fontFamily: "Inter, sans-serif",
      fontSize: "26px",
      fontStyle: "bold",
      stroke: "#020617",
      strokeThickness: 6,
    })
    .setOrigin(0.5);
  const caption = scene.add
    .text(WORLD_WIDTH / 2, 92, "The ravine claims another contender", {
      color: "#9bd9ff",
      fontFamily: "Inter, sans-serif",
      fontSize: "15px",
    })
    .setOrigin(0.5);
  root.add([heading, caption]);

  const character = scene.add.container(270, 145).setDepth(1_005);
  const pink = 0xf472b6;
  const leftArm = scene.add.ellipse(-43, 35, 38, 17, pink).setRotation(-0.35);
  const rightArm = scene.add.ellipse(43, 35, 38, 17, pink).setRotation(0.35);
  const leftFoot = scene.add.ellipse(-24, 79, 42, 20, 0xef5da8);
  const rightFoot = scene.add.ellipse(24, 79, 42, 20, 0xef5da8);
  const body = scene.add
    .circle(0, 40, 45, pink)
    .setStrokeStyle(5, 0xfbcfe8, 0.9);
  const headFrame = scene.add
    .circle(0, -4, 35, 0x1e293b)
    .setStrokeStyle(4, 0xffffff, 0.95);
  const initials = scene.add
    .text(0, -4, initialsFor(loser.name), {
      color: "#ffffff",
      fontFamily: "Inter, sans-serif",
      fontSize: "25px",
      fontStyle: "bold",
    })
    .setOrigin(0.5);
  const nameplate = scene.add
    .text(0, 109, loser.name, {
      backgroundColor: "#020617cc",
      color: "#ffffff",
      fontFamily: "Inter, sans-serif",
      fontSize: "15px",
      fontStyle: "bold",
      padding: { x: 9, y: 4 },
    })
    .setOrigin(0.5);
  character.add([
    leftArm,
    rightArm,
    leftFoot,
    rightFoot,
    body,
    headFrame,
    initials,
    nameplate,
  ]);
  root.add(character);

  const finish = () => {
    if (!active) return;
    active = false;
    scene.tweens.killTweensOf(character);
    scene.tweens.killTweensOf(root);
    stopYell();
    root.destroy(true);
    if (avatarKey && scene.textures.exists(avatarKey)) {
      scene.textures.remove(avatarKey);
    }
    resolveFinished();
  };

  addAvatarWhenReady(scene, character, initials, loser.avatarUrl, (key) => {
    avatarKey = key;
    return active;
  });

  if (reducedMotion) {
    stopYell = startArtilleryRavineYell();
    scene.tweens.add({
      targets: character,
      alpha: 0,
      duration: 450,
      ease: "Quad.easeIn",
      scaleX: 0.35,
      scaleY: 0.35,
      x: 470,
      y: 485,
      onComplete: finish,
    });
  } else {
    scene.tweens.add({
      targets: character,
      duration: 650,
      ease: "Sine.easeInOut",
      x: 365,
      y: 150,
      onComplete: () => {
        if (!active) return;
        scene.tweens.add({
          targets: character,
          angle: 14,
          duration: 165,
          ease: "Sine.easeInOut",
          yoyo: true,
          repeat: 2,
          onComplete: () => {
            stopYell = startArtilleryRavineYell();
            tumbleToFirstLedge(scene, character, root, finish);
          },
        });
      },
    });
  }

  return { finished, skip: finish };
}

function tumbleToFirstLedge(
  scene: Phaser.Scene,
  character: Phaser.GameObjects.Container,
  root: Phaser.GameObjects.Container,
  finish: () => void,
) {
  scene.cameras.main.shake(130, 0.004);
  scene.tweens.add({
    targets: character,
    angle: 115,
    duration: 510,
    ease: "Quad.easeIn",
    scaleX: 0.88,
    scaleY: 0.88,
    x: 450,
    y: 295,
    onComplete: () => {
      burstDust(scene, root, 438, 320);
      scene.tweens.add({
        targets: character,
        angle: 245,
        duration: 560,
        ease: "Cubic.easeIn",
        scaleX: 0.68,
        scaleY: 0.68,
        x: 565,
        y: 388,
        onComplete: () => {
          burstDust(scene, root, 577, 396);
          scene.tweens.add({
            targets: character,
            alpha: 0.18,
            angle: 510,
            duration: 780,
            ease: "Cubic.easeIn",
            scaleX: 0.12,
            scaleY: 0.12,
            x: 482,
            y: 565,
            onComplete: () => {
              scene.tweens.add({
                targets: root,
                alpha: 0,
                duration: 280,
                onComplete: finish,
              });
            },
          });
        },
      });
    },
  });
}

function burstDust(
  scene: Phaser.Scene,
  root: Phaser.GameObjects.Container,
  x: number,
  y: number,
) {
  for (let index = 0; index < 8; index += 1) {
    const dust = scene.add
      .circle(x, y, 4 + (index % 3), 0x9db5b1, 0.72)
      .setDepth(1_004);
    root.add(dust);
    const angle = (Math.PI * 2 * index) / 8;
    scene.tweens.add({
      targets: dust,
      alpha: 0,
      duration: 420,
      scale: 1.7,
      x: x + Math.cos(angle) * (25 + (index % 2) * 9),
      y: y + Math.sin(angle) * 18,
    });
  }
}

function addAvatarWhenReady(
  scene: Phaser.Scene,
  character: Phaser.GameObjects.Container,
  initials: Phaser.GameObjects.Text,
  avatarUrl: string | null,
  keepTexture: (key: string) => boolean,
) {
  const url = avatarUrl?.trim();
  if (!url) return;

  const key = `artillery-ravine-avatar-${Date.now()}-${Math.random()}`;
  scene.load.once(`filecomplete-image-${key}`, () => {
    if (!keepTexture(key) || !character.active) {
      if (scene.textures.exists(key)) scene.textures.remove(key);
      return;
    }
    const maskShape = scene.add.circle(0, -4, 30, 0xffffff).setVisible(false);
    const avatar = scene.add.image(0, -4, key).setDisplaySize(60, 60);
    avatar.setMask(maskShape.createGeometryMask());
    character.add([maskShape, avatar]);
    initials.setVisible(false);
  });
  scene.load.image(key, url);
  if (!scene.load.isLoading()) scene.load.start();
}

function initialsFor(name: string) {
  const initials = name
    .trim()
    .split(/\s+/u)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? "")
    .join("");
  return initials || "?";
}

function fillPolygon(
  graphics: Phaser.GameObjects.Graphics,
  points: Array<readonly [number, number]>,
) {
  const [first, ...rest] = points;
  if (!first) return;
  graphics.beginPath();
  graphics.moveTo(first[0], first[1]);
  for (const point of rest) graphics.lineTo(point[0], point[1]);
  graphics.closePath();
  graphics.fillPath();
}
