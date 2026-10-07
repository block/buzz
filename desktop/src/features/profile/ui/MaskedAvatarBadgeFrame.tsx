import type * as React from "react";

import { motion } from "motion/react";

import { cn } from "@/shared/lib/cn";

export type AvatarBadgeCircle = {
  cx: number;
  cy: number;
  r: number;
};

export type AvatarBadgeBox = {
  bottom: number;
  height: number;
  right: number;
  width: number;
};

type Point = {
  x: number;
  y: number;
};

type RoundedAvatarMaskGeometry = {
  avatar: AvatarBadgeCircle;
  avatarLower: Point;
  avatarUpper: Point;
  cutout: AvatarBadgeCircle;
  cutoutLower: Point;
  cutoutUpper: Point;
  lowerHandleLength: number;
  upperHandleLength: number;
};

type BadgeMotionTarget = {
  height: string;
  left: string;
  top: string;
  width: string;
};

type MaskedAvatarBadgeFrameProps = {
  badge?: React.ReactNode;
  badgeBox?: AvatarBadgeBox;
  badgeClassName?: string;
  children: React.ReactNode;
  className?: string;
  clipTestId?: string;
  cornerRadius?: number;
  curve?: AvatarBadgeCurve;
  cutout?: AvatarBadgeCircle;
  cutoutWidth?: number;
  maskMode?: "clip-path" | "radial";
  maskTransition?: React.ComponentProps<typeof motion.path>["transition"];
  size: number;
};

export type AvatarBadgeCurve = Partial<{
  avatarRoundingAngle: number;
  cutoutRoundingLength: number;
  cutoutRoundingMinAngle: number;
  cutoutRoundingMaxAngle: number;
  handleDistanceRatio: number;
  handleLengthRatio: number;
}>;

const DEFAULT_AVATAR_BADGE_CURVE = {
  avatarRoundingAngle: 0.075,
  cutoutRoundingLength: 5.5,
  cutoutRoundingMinAngle: 0.22,
  cutoutRoundingMaxAngle: 0.38,
  handleDistanceRatio: 0.42,
  handleLengthRatio: 0.14,
} as const;

export const STATUS_DOT_MASK_CURVE = {
  avatarRoundingAngle: 0.11,
  cutoutRoundingLength: 6.5,
  cutoutRoundingMinAngle: 0.28,
  cutoutRoundingMaxAngle: 0.44,
  handleDistanceRatio: 0.5,
  handleLengthRatio: 0.2,
} satisfies AvatarBadgeCurve;

function getCircleIntersections(
  avatar: AvatarBadgeCircle,
  cutout: AvatarBadgeCircle,
): [Point, Point] {
  const dx = cutout.cx - avatar.cx;
  const dy = cutout.cy - avatar.cy;
  const distance = Math.hypot(dx, dy);
  const distanceToMidpoint =
    (avatar.r * avatar.r - cutout.r * cutout.r + distance * distance) /
    (2 * distance);
  const halfChord = Math.sqrt(
    Math.max(0, avatar.r * avatar.r - distanceToMidpoint * distanceToMidpoint),
  );
  const ux = dx / distance;
  const uy = dy / distance;
  const midpoint = {
    x: avatar.cx + distanceToMidpoint * ux,
    y: avatar.cy + distanceToMidpoint * uy,
  };
  const perpendicular = { x: -uy, y: ux };

  return [
    {
      x: midpoint.x + halfChord * perpendicular.x,
      y: midpoint.y + halfChord * perpendicular.y,
    },
    {
      x: midpoint.x - halfChord * perpendicular.x,
      y: midpoint.y - halfChord * perpendicular.y,
    },
  ];
}

function getAngle(circle: AvatarBadgeCircle, point: Point) {
  return Math.atan2(point.y - circle.cy, point.x - circle.cx);
}

function getPointOnCircle(circle: AvatarBadgeCircle, angle: number): Point {
  return {
    x: circle.cx + circle.r * Math.cos(angle),
    y: circle.cy + circle.r * Math.sin(angle),
  };
}

function getTangent(angle: number, direction: 1 | -1): Point {
  return direction === 1
    ? { x: -Math.sin(angle), y: Math.cos(angle) }
    : { x: Math.sin(angle), y: -Math.cos(angle) };
}

function getControlPoint(
  point: Point,
  tangent: Point,
  distance: number,
): Point {
  return {
    x: point.x + tangent.x * distance,
    y: point.y + tangent.y * distance,
  };
}

function getDistance(a: Point, b: Point) {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

function toRem(value: number) {
  return `${value / 16}rem`;
}

function toPercent(value: number) {
  return `${value * 100}%`;
}

function getBadgeMotionTarget(
  size: number,
  badgeBox: AvatarBadgeBox,
  cutout: AvatarBadgeCircle,
): BadgeMotionTarget {
  return {
    height: toPercent(badgeBox.height / size),
    left: toPercent(cutout.cx / size),
    top: toPercent(cutout.cy / size),
    width: toPercent(badgeBox.width / size),
  };
}

function getBadgeStyle(target: BadgeMotionTarget): React.CSSProperties {
  return {
    ...target,
    transform: "translate3d(-50%, -50%, 0)",
    transformOrigin: "center",
  };
}

function getRoundedAvatarMaskGeometry(
  size: number,
  cutout: AvatarBadgeCircle,
  curve?: AvatarBadgeCurve,
  avatarOverride?: AvatarBadgeCircle,
): RoundedAvatarMaskGeometry {
  const resolvedCurve = { ...DEFAULT_AVATAR_BADGE_CURVE, ...curve };
  const avatar = avatarOverride ?? {
    cx: size / 2,
    cy: size / 2,
    r: size / 2,
  };
  const [firstIntersection, secondIntersection] = getCircleIntersections(
    avatar,
    cutout,
  );
  const upperIntersection =
    firstIntersection.y < secondIntersection.y
      ? firstIntersection
      : secondIntersection;
  const lowerIntersection =
    firstIntersection.y < secondIntersection.y
      ? secondIntersection
      : firstIntersection;
  const cutoutRoundingAngle = Math.min(
    resolvedCurve.cutoutRoundingMaxAngle,
    Math.max(
      resolvedCurve.cutoutRoundingMinAngle,
      resolvedCurve.cutoutRoundingLength / cutout.r,
    ),
  );

  const avatarUpper = getPointOnCircle(
    avatar,
    getAngle(avatar, upperIntersection) - resolvedCurve.avatarRoundingAngle,
  );
  const avatarLower = getPointOnCircle(
    avatar,
    getAngle(avatar, lowerIntersection) + resolvedCurve.avatarRoundingAngle,
  );
  const cutoutUpper = getPointOnCircle(
    cutout,
    getAngle(cutout, upperIntersection) - cutoutRoundingAngle,
  );
  const cutoutLower = getPointOnCircle(
    cutout,
    getAngle(cutout, lowerIntersection) + cutoutRoundingAngle,
  );

  const upperHandleLength = Math.min(
    cutout.r * resolvedCurve.handleLengthRatio,
    getDistance(cutoutUpper, avatarUpper) * resolvedCurve.handleDistanceRatio,
  );
  const lowerHandleLength = Math.min(
    cutout.r * resolvedCurve.handleLengthRatio,
    getDistance(avatarLower, cutoutLower) * resolvedCurve.handleDistanceRatio,
  );
  return {
    avatar,
    avatarLower,
    avatarUpper,
    cutout,
    cutoutLower,
    cutoutUpper,
    lowerHandleLength,
    upperHandleLength,
  };
}

function getCubicPoint(
  start: Point,
  firstControl: Point,
  secondControl: Point,
  end: Point,
  progress: number,
): Point {
  const remaining = 1 - progress;

  return {
    x:
      remaining * remaining * remaining * start.x +
      3 * remaining * remaining * progress * firstControl.x +
      3 * remaining * progress * progress * secondControl.x +
      progress * progress * progress * end.x,
    y:
      remaining * remaining * remaining * start.y +
      3 * remaining * remaining * progress * firstControl.y +
      3 * remaining * progress * progress * secondControl.y +
      progress * progress * progress * end.y,
  };
}

function sampleCubic(
  start: Point,
  firstControl: Point,
  secondControl: Point,
  end: Point,
  segments: number,
) {
  return Array.from({ length: segments }, (_, index) =>
    getCubicPoint(
      start,
      firstControl,
      secondControl,
      end,
      (index + 1) / segments,
    ),
  );
}

function getArcSweep(
  startAngle: number,
  endAngle: number,
  direction: 1 | -1,
  largeArc: boolean,
) {
  const fullTurn = Math.PI * 2;

  if (direction === 1) {
    let sweep = endAngle - startAngle;
    while (sweep < 0) sweep += fullTurn;
    if (largeArc && sweep < Math.PI) sweep += fullTurn;
    if (!largeArc && sweep > Math.PI) sweep -= fullTurn;
    return sweep;
  }

  let sweep = startAngle - endAngle;
  while (sweep < 0) sweep += fullTurn;
  if (largeArc && sweep < Math.PI) sweep += fullTurn;
  if (!largeArc && sweep > Math.PI) sweep -= fullTurn;
  return -sweep;
}

function sampleArc(
  circle: AvatarBadgeCircle,
  startAngle: number,
  endAngle: number,
  direction: 1 | -1,
  largeArc: boolean,
  segments: number,
) {
  const sweep = getArcSweep(startAngle, endAngle, direction, largeArc);

  return Array.from({ length: segments }, (_, index) =>
    getPointOnCircle(circle, startAngle + sweep * ((index + 1) / segments)),
  );
}

function sampleStableOuterBoundary(
  avatar: AvatarBadgeCircle,
  startAngle: number,
  endAngle: number,
  direction: 1 | -1,
  largeArc: boolean,
  segments: number,
) {
  const outerBoundary = { ...avatar, r: avatar.r * 4 };

  return [
    getPointOnCircle(outerBoundary, startAngle),
    ...sampleArc(
      outerBoundary,
      startAngle,
      endAngle,
      direction,
      largeArc,
      segments - 2,
    ),
    getPointOnCircle(avatar, endAngle),
  ];
}

function toPolygonPoint(point: Point, size: number) {
  return `${toPercent(point.x / size)} ${toPercent(point.y / size)}`;
}

function getRoundedAvatarMaskPolygon(
  size: number,
  cutout: AvatarBadgeCircle,
  curve?: AvatarBadgeCurve,
  stabilizeOuterBoundary = false,
) {
  const {
    avatar,
    avatarLower,
    avatarUpper,
    cutout: resolvedCutout,
    cutoutLower,
    cutoutUpper,
    lowerHandleLength,
    upperHandleLength,
  } = getRoundedAvatarMaskGeometry(size, cutout, curve);
  const upperCutoutTangent = getTangent(
    getAngle(resolvedCutout, cutoutUpper),
    1,
  );
  const upperAvatarTangent = getTangent(getAngle(avatar, avatarUpper), -1);
  const lowerAvatarTangent = getTangent(getAngle(avatar, avatarLower), -1);
  const lowerCutoutTangent = getTangent(
    getAngle(resolvedCutout, cutoutLower),
    1,
  );
  const points = [
    cutoutUpper,
    ...sampleCubic(
      cutoutUpper,
      getControlPoint(cutoutUpper, upperCutoutTangent, upperHandleLength),
      getControlPoint(avatarUpper, upperAvatarTangent, -upperHandleLength),
      avatarUpper,
      12,
    ),
    ...(stabilizeOuterBoundary
      ? sampleStableOuterBoundary(
          avatar,
          getAngle(avatar, avatarUpper),
          getAngle(avatar, avatarLower),
          -1,
          true,
          96,
        )
      : sampleArc(
          avatar,
          getAngle(avatar, avatarUpper),
          getAngle(avatar, avatarLower),
          -1,
          true,
          96,
        )),
    ...sampleCubic(
      avatarLower,
      getControlPoint(avatarLower, lowerAvatarTangent, lowerHandleLength),
      getControlPoint(cutoutLower, lowerCutoutTangent, -lowerHandleLength),
      cutoutLower,
      12,
    ),
    ...sampleArc(
      resolvedCutout,
      getAngle(resolvedCutout, cutoutLower),
      getAngle(resolvedCutout, cutoutUpper),
      1,
      false,
      24,
    ),
  ];

  return `polygon(${points.map((point) => toPolygonPoint(point, size)).join(", ")})`;
}

function getRoundedAvatarCapsuleMaskPolygon(
  size: number,
  cutout: AvatarBadgeCircle,
  cutoutWidth: number,
  curve?: AvatarBadgeCurve,
  stabilizeOuterBoundary = false,
) {
  const resolvedCurve = { ...DEFAULT_AVATAR_BADGE_CURVE, ...curve };
  const avatar = {
    cx: size / 2,
    cy: size / 2,
    r: size / 2,
  };
  const straightHalfWidth = Math.max(0, cutoutWidth / 2 - cutout.r);
  const leftCap = {
    cx: cutout.cx - straightHalfWidth,
    cy: cutout.cy,
    r: cutout.r,
  };
  const rightCap = {
    cx: cutout.cx + straightHalfWidth,
    cy: cutout.cy,
    r: cutout.r,
  };
  const leftIntersection = getCircleIntersections(avatar, leftCap).reduce(
    (leftmost, point) => (point.x < leftmost.x ? point : leftmost),
  );
  const rightIntersection = getCircleIntersections(avatar, rightCap).reduce(
    (rightmost, point) => (point.x > rightmost.x ? point : rightmost),
  );
  const cutoutRoundingAngle = Math.min(
    resolvedCurve.cutoutRoundingMaxAngle,
    Math.max(
      resolvedCurve.cutoutRoundingMinAngle,
      resolvedCurve.cutoutRoundingLength / cutout.r,
    ),
  );
  const avatarLeft = getPointOnCircle(
    avatar,
    getAngle(avatar, leftIntersection) + resolvedCurve.avatarRoundingAngle,
  );
  const avatarRight = getPointOnCircle(
    avatar,
    getAngle(avatar, rightIntersection) - resolvedCurve.avatarRoundingAngle,
  );
  const cutoutLeft = getPointOnCircle(
    leftCap,
    getAngle(leftCap, leftIntersection) + cutoutRoundingAngle,
  );
  const cutoutRight = getPointOnCircle(
    rightCap,
    getAngle(rightCap, rightIntersection) - cutoutRoundingAngle,
  );
  const leftHandleLength = Math.min(
    cutout.r * resolvedCurve.handleLengthRatio,
    getDistance(cutoutLeft, avatarLeft) * resolvedCurve.handleDistanceRatio,
  );
  const rightHandleLength = Math.min(
    cutout.r * resolvedCurve.handleLengthRatio,
    getDistance(avatarRight, cutoutRight) * resolvedCurve.handleDistanceRatio,
  );
  const cutoutLeftTangent = getTangent(getAngle(leftCap, cutoutLeft), -1);
  const avatarLeftTangent = getTangent(getAngle(avatar, avatarLeft), 1);
  const avatarRightTangent = getTangent(getAngle(avatar, avatarRight), 1);
  const cutoutRightTangent = getTangent(getAngle(rightCap, cutoutRight), -1);
  const points = [
    cutoutLeft,
    ...sampleCubic(
      cutoutLeft,
      getControlPoint(cutoutLeft, cutoutLeftTangent, leftHandleLength),
      getControlPoint(avatarLeft, avatarLeftTangent, -leftHandleLength),
      avatarLeft,
      12,
    ),
    ...(stabilizeOuterBoundary
      ? sampleStableOuterBoundary(
          avatar,
          getAngle(avatar, avatarLeft),
          getAngle(avatar, avatarRight),
          1,
          true,
          96,
        )
      : sampleArc(
          avatar,
          getAngle(avatar, avatarLeft),
          getAngle(avatar, avatarRight),
          1,
          true,
          96,
        )),
    ...sampleCubic(
      avatarRight,
      getControlPoint(avatarRight, avatarRightTangent, rightHandleLength),
      getControlPoint(cutoutRight, cutoutRightTangent, -rightHandleLength),
      cutoutRight,
      12,
    ),
    ...sampleArc(
      rightCap,
      getAngle(rightCap, cutoutRight),
      -Math.PI / 2,
      -1,
      false,
      12,
    ),
    { x: leftCap.cx, y: cutout.cy - cutout.r },
    ...sampleArc(
      leftCap,
      -Math.PI / 2,
      getAngle(leftCap, cutoutLeft),
      -1,
      false,
      11,
    ),
  ];

  // Keep the capsule contour aligned with the circular status cutout's point
  // order. Matching like-for-like edges prevents the polygon from folding
  // across the avatar while Motion interpolates between the two shapes.
  const joinSegments = 12;
  const outerSegments = 96;
  const outerEndIndex = joinSegments + outerSegments;
  const rightJoinEndIndex = joinSegments * 2 + outerSegments;
  const alignedPoints = [
    points[rightJoinEndIndex],
    ...points.slice(outerEndIndex, rightJoinEndIndex).reverse(),
    ...points.slice(joinSegments, outerEndIndex).reverse(),
    ...points.slice(0, joinSegments).reverse(),
    ...points.slice(rightJoinEndIndex + 1).reverse(),
  ];

  return `polygon(${alignedPoints.map((point) => toPolygonPoint(point, size)).join(", ")})`;
}

function getRoundedSquareMaskPolygon(
  size: number,
  cornerRadius: number,
  cutout: AvatarBadgeCircle,
  curve?: AvatarBadgeCurve,
) {
  const radius = Math.min(Math.max(cornerRadius, 0), size / 2);
  const bottomRightCorner = {
    cx: size - radius,
    cy: size - radius,
    r: radius,
  };
  const {
    avatar: resolvedCorner,
    avatarLower,
    avatarUpper,
    cutout: resolvedCutout,
    cutoutLower,
    cutoutUpper,
    lowerHandleLength,
    upperHandleLength,
  } = getRoundedAvatarMaskGeometry(size, cutout, curve, bottomRightCorner);
  const upperCutoutTangent = getTangent(
    getAngle(resolvedCutout, cutoutUpper),
    1,
  );
  const upperAvatarTangent = getTangent(
    getAngle(resolvedCorner, avatarUpper),
    -1,
  );
  const lowerAvatarTangent = getTangent(
    getAngle(resolvedCorner, avatarLower),
    -1,
  );
  const lowerCutoutTangent = getTangent(
    getAngle(resolvedCutout, cutoutLower),
    1,
  );
  const topRightCorner = { cx: size - radius, cy: radius, r: radius };
  const topLeftCorner = { cx: radius, cy: radius, r: radius };
  const bottomLeftCorner = { cx: radius, cy: size - radius, r: radius };
  const outerPoints = [
    ...sampleArc(
      resolvedCorner,
      getAngle(resolvedCorner, avatarUpper),
      0,
      -1,
      false,
      8,
    ),
    { x: size, y: radius },
    ...sampleArc(topRightCorner, 0, -Math.PI / 2, -1, false, 16),
    { x: radius, y: 0 },
    ...sampleArc(topLeftCorner, -Math.PI / 2, -Math.PI, -1, false, 16),
    { x: 0, y: size - radius },
    ...sampleArc(bottomLeftCorner, Math.PI, Math.PI / 2, -1, false, 16),
    { x: size - radius, y: size },
    ...sampleArc(
      resolvedCorner,
      Math.PI / 2,
      getAngle(resolvedCorner, avatarLower),
      -1,
      false,
      8,
    ),
  ];
  const points = [
    cutoutUpper,
    ...sampleCubic(
      cutoutUpper,
      getControlPoint(cutoutUpper, upperCutoutTangent, upperHandleLength),
      getControlPoint(avatarUpper, upperAvatarTangent, -upperHandleLength),
      avatarUpper,
      12,
    ),
    ...outerPoints,
    ...sampleCubic(
      avatarLower,
      getControlPoint(avatarLower, lowerAvatarTangent, lowerHandleLength),
      getControlPoint(cutoutLower, lowerCutoutTangent, -lowerHandleLength),
      cutoutLower,
      12,
    ),
    ...sampleArc(
      resolvedCutout,
      getAngle(resolvedCutout, cutoutLower),
      getAngle(resolvedCutout, cutoutUpper),
      1,
      true,
      48,
    ),
  ];

  return `polygon(${points.map((point) => toPolygonPoint(point, size)).join(", ")})`;
}

export function MaskedAvatarBadgeFrame({
  badge,
  badgeBox,
  badgeClassName,
  children,
  className,
  clipTestId,
  cornerRadius,
  curve,
  cutout,
  cutoutWidth,
  maskMode = "clip-path",
  maskTransition,
  size,
}: MaskedAvatarBadgeFrameProps) {
  const shouldMask = Boolean(badge && badgeBox && cutout);
  const stabilizeOuterBoundary = Boolean(maskTransition);
  const maskPolygon = cutout
    ? cornerRadius === undefined
      ? cutoutWidth && cutoutWidth > cutout.r * 2
        ? getRoundedAvatarCapsuleMaskPolygon(
            size,
            cutout,
            cutoutWidth,
            curve,
            stabilizeOuterBoundary,
          )
        : getRoundedAvatarMaskPolygon(
            size,
            cutout,
            curve,
            stabilizeOuterBoundary,
          )
      : getRoundedSquareMaskPolygon(size, cornerRadius, cutout, curve)
    : undefined;
  const radialMask =
    maskMode === "radial" && cutout
      ? `radial-gradient(circle ${toRem(cutout.r)} at ${toRem(
          cutout.cx,
        )} ${toRem(
          cutout.cy,
        )}, transparent calc(100% - 0.03125rem), black 100%)`
      : undefined;
  const sizeStyle = { height: toRem(size), width: toRem(size) };
  const badgeMotionTarget =
    badgeBox && cutout
      ? getBadgeMotionTarget(size, badgeBox, cutout)
      : undefined;
  const badgeStyle = badgeMotionTarget
    ? getBadgeStyle(badgeMotionTarget)
    : undefined;

  if (!shouldMask) {
    return (
      <div className={cn("relative shrink-0", className)} style={sizeStyle}>
        {children}
      </div>
    );
  }

  return (
    <div className={cn("relative shrink-0", className)} style={sizeStyle}>
      <motion.div
        animate={
          maskTransition && !radialMask ? { clipPath: maskPolygon } : undefined
        }
        className="h-full w-full"
        data-testid={clipTestId}
        initial={false}
        style={{
          // WebKit otherwise applies the prefixed path immediately while the
          // unprefixed path is still animating, which briefly tears the avatar.
          WebkitClipPath:
            radialMask || maskTransition ? undefined : maskPolygon,
          WebkitMaskImage: radialMask,
          backfaceVisibility:
            maskTransition && !radialMask ? "hidden" : undefined,
          clipPath: radialMask ? undefined : maskPolygon,
          maskImage: radialMask,
          transform:
            maskTransition && !radialMask ? "translateZ(0)" : undefined,
          willChange: maskTransition && !radialMask ? "clip-path" : undefined,
        }}
        transition={maskTransition}
      >
        {children}
      </motion.div>

      <motion.span
        animate={maskTransition ? badgeMotionTarget : undefined}
        className={cn(
          "absolute z-20 flex items-center justify-center rounded-full",
          badgeClassName,
        )}
        initial={false}
        style={badgeStyle}
        transition={maskTransition}
      >
        {badge}
      </motion.span>
    </div>
  );
}
