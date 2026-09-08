import 'dart:math' as math;

import 'package:flutter/widgets.dart';

/// Avatar-with-badge geometry ported from desktop's `MaskedAvatarBadgeFrame`
/// (`desktop/src/features/profile/ui/MaskedAvatarBadgeFrame.tsx`).
///
/// A badge (presence dot, status glyph) sits in a notch cut out of the avatar
/// rather than on top of it. The notch is not a plain circular subtraction: the
/// two edges are joined by cubic fillets so the avatar's outline flows into the
/// cutout instead of meeting it at a cusp. That fillet is the whole visual
/// signature, which is why the curve constants are ported verbatim.
@immutable
class AvatarBadgeCurve {
  const AvatarBadgeCurve({
    this.avatarRoundingAngle = 0.075,
    this.cutoutRoundingLength = 5.5,
    this.cutoutRoundingMinAngle = 0.22,
    this.cutoutRoundingMaxAngle = 0.38,
    this.handleDistanceRatio = 0.42,
    this.handleLengthRatio = 0.14,
  });

  /// Radians the avatar's edge pulls back from the intersection before the
  /// fillet starts.
  final double avatarRoundingAngle;

  /// Fillet length along the cutout, converted to an angle by dividing by the
  /// cutout radius and clamped between the two angle bounds below.
  final double cutoutRoundingLength;
  final double cutoutRoundingMinAngle;
  final double cutoutRoundingMaxAngle;

  /// Bezier handle length, as a fraction of the gap between the two fillet
  /// endpoints and of the cutout radius — whichever is shorter wins.
  final double handleDistanceRatio;
  final double handleLengthRatio;

  /// Desktop's `DEFAULT_AVATAR_BADGE_CURVE`, used for large badges.
  static const standard = AvatarBadgeCurve();

  /// Desktop's `STATUS_DOT_MASK_CURVE` — a rounder, softer notch that reads
  /// correctly at presence-dot sizes.
  static const statusDot = AvatarBadgeCurve(
    avatarRoundingAngle: 0.11,
    cutoutRoundingLength: 6.5,
    cutoutRoundingMinAngle: 0.28,
    cutoutRoundingMaxAngle: 0.44,
    handleDistanceRatio: 0.5,
    handleLengthRatio: 0.2,
  );

  @override
  bool operator ==(Object other) =>
      other is AvatarBadgeCurve &&
      other.avatarRoundingAngle == avatarRoundingAngle &&
      other.cutoutRoundingLength == cutoutRoundingLength &&
      other.cutoutRoundingMinAngle == cutoutRoundingMinAngle &&
      other.cutoutRoundingMaxAngle == cutoutRoundingMaxAngle &&
      other.handleDistanceRatio == handleDistanceRatio &&
      other.handleLengthRatio == handleLengthRatio;

  @override
  int get hashCode => Object.hash(
    avatarRoundingAngle,
    cutoutRoundingLength,
    cutoutRoundingMinAngle,
    cutoutRoundingMaxAngle,
    handleDistanceRatio,
    handleLengthRatio,
  );
}

/// Where the notch sits and how big the badge in it is, as fractions of the
/// avatar box so one constant covers every avatar size.
@immutable
class AvatarBadgeMaskGeometry {
  const AvatarBadgeMaskGeometry({
    required this.cutoutCenterRatio,
    required this.cutoutRadiusRatio,
    required this.badgeSizeRatio,
    required this.curve,
  });

  /// Distance of the notch centre from the top-left, on both axes.
  final double cutoutCenterRatio;
  final double cutoutRadiusRatio;

  /// The badge is centred on the notch and is deliberately smaller than it, so
  /// a ring of background shows between badge and avatar.
  final double badgeSizeRatio;

  final AvatarBadgeCurve curve;

  /// Desktop's settings-card treatment: a 192px avatar with a 54px badge notched
  /// in at (165, 165) with radius 30.
  static const badge = AvatarBadgeMaskGeometry(
    cutoutCenterRatio: 165 / 192,
    cutoutRadiusRatio: 30 / 192,
    badgeSizeRatio: 54 / 192,
    curve: AvatarBadgeCurve.standard,
  );

  /// Desktop's sidebar presence treatment: a 32px avatar with a 14px dot frame
  /// notched in at (28, 28) with radius 7.5.
  static const presenceDot = AvatarBadgeMaskGeometry(
    cutoutCenterRatio: 28 / 32,
    cutoutRadiusRatio: 7.5 / 32,
    badgeSizeRatio: 14 / 32,
    curve: AvatarBadgeCurve.statusDot,
  );

  Offset cutoutCenter(double size) =>
      Offset(size * cutoutCenterRatio, size * cutoutCenterRatio);

  double cutoutRadius(double size) => size * cutoutRadiusRatio;

  double badgeSize(double size) => size * badgeSizeRatio;

  @override
  bool operator ==(Object other) =>
      other is AvatarBadgeMaskGeometry &&
      other.cutoutCenterRatio == cutoutCenterRatio &&
      other.cutoutRadiusRatio == cutoutRadiusRatio &&
      other.badgeSizeRatio == badgeSizeRatio &&
      other.curve == curve;

  @override
  int get hashCode =>
      Object.hash(cutoutCenterRatio, cutoutRadiusRatio, badgeSizeRatio, curve);
}

double _angleTo(Offset center, Offset point) =>
    math.atan2(point.dy - center.dy, point.dx - center.dx);

Offset _pointOn(Offset center, double radius, double angle) =>
    center + Offset(math.cos(angle), math.sin(angle)) * radius;

/// Unit tangent at [angle]; [clockwise] picks which way around the circle.
Offset _tangent(double angle, {required bool clockwise}) => clockwise
    ? Offset(-math.sin(angle), math.cos(angle))
    : Offset(math.sin(angle), -math.cos(angle));

/// Signed sweep from [startAngle] to [endAngle], ported from desktop's
/// `getArcSweep`. Flutter shares SVG's angle convention (y grows downward, so a
/// positive sweep runs clockwise on screen), which is why the port is direct.
double _arcSweep(
  double startAngle,
  double endAngle, {
  required bool clockwise,
  required bool largeArc,
}) {
  const fullTurn = math.pi * 2;
  var sweep = clockwise ? endAngle - startAngle : startAngle - endAngle;
  while (sweep < 0) {
    sweep += fullTurn;
  }
  if (largeArc && sweep < math.pi) sweep += fullTurn;
  if (!largeArc && sweep > math.pi) sweep -= fullTurn;
  return clockwise ? sweep : -sweep;
}

/// The avatar outline with [geometry]'s notch cut out of its bottom-right.
///
/// Falls back to a plain circle when the notch does not actually cross the
/// avatar's edge — there is nothing to fillet in that case.
Path avatarBadgeMaskPath({
  required double size,
  required AvatarBadgeMaskGeometry geometry,
}) {
  final avatarRadius = size / 2;
  final avatarCenter = Offset(avatarRadius, avatarRadius);
  final avatarRect = Rect.fromCircle(
    center: avatarCenter,
    radius: avatarRadius,
  );
  final cutoutCenter = geometry.cutoutCenter(size);
  final cutoutRadius = geometry.cutoutRadius(size);
  final curve = geometry.curve;

  final span = cutoutCenter - avatarCenter;
  final distance = span.distance;
  if (distance >= avatarRadius + cutoutRadius ||
      distance <= (avatarRadius - cutoutRadius).abs()) {
    return Path()..addOval(avatarRect);
  }

  // Chord where the two circles cross, split into the upper and lower crossing.
  final distanceToMidpoint =
      (avatarRadius * avatarRadius -
          cutoutRadius * cutoutRadius +
          distance * distance) /
      (2 * distance);
  final halfChord = math.sqrt(
    math.max(
      0,
      avatarRadius * avatarRadius - distanceToMidpoint * distanceToMidpoint,
    ),
  );
  final unit = span / distance;
  final midpoint = avatarCenter + unit * distanceToMidpoint;
  final perpendicular = Offset(-unit.dy, unit.dx);
  final first = midpoint + perpendicular * halfChord;
  final second = midpoint - perpendicular * halfChord;
  final upperCrossing = first.dy < second.dy ? first : second;
  final lowerCrossing = first.dy < second.dy ? second : first;

  final cutoutRoundingAngle = math.min(
    curve.cutoutRoundingMaxAngle,
    math.max(
      curve.cutoutRoundingMinAngle,
      curve.cutoutRoundingLength / cutoutRadius,
    ),
  );

  // Pull each edge back from its crossing; the gap left behind is the fillet.
  final avatarUpperAngle =
      _angleTo(avatarCenter, upperCrossing) - curve.avatarRoundingAngle;
  final avatarLowerAngle =
      _angleTo(avatarCenter, lowerCrossing) + curve.avatarRoundingAngle;
  final cutoutUpperAngle =
      _angleTo(cutoutCenter, upperCrossing) - cutoutRoundingAngle;
  final cutoutLowerAngle =
      _angleTo(cutoutCenter, lowerCrossing) + cutoutRoundingAngle;

  final avatarUpper = _pointOn(avatarCenter, avatarRadius, avatarUpperAngle);
  final avatarLower = _pointOn(avatarCenter, avatarRadius, avatarLowerAngle);
  final cutoutUpper = _pointOn(cutoutCenter, cutoutRadius, cutoutUpperAngle);
  final cutoutLower = _pointOn(cutoutCenter, cutoutRadius, cutoutLowerAngle);

  final upperHandle = math.min(
    cutoutRadius * curve.handleLengthRatio,
    (cutoutUpper - avatarUpper).distance * curve.handleDistanceRatio,
  );
  final lowerHandle = math.min(
    cutoutRadius * curve.handleLengthRatio,
    (avatarLower - cutoutLower).distance * curve.handleDistanceRatio,
  );

  final upperFromCutout =
      cutoutUpper + _tangent(cutoutUpperAngle, clockwise: true) * upperHandle;
  final upperIntoAvatar =
      avatarUpper - _tangent(avatarUpperAngle, clockwise: false) * upperHandle;
  final lowerFromAvatar =
      avatarLower + _tangent(avatarLowerAngle, clockwise: false) * lowerHandle;
  final lowerIntoCutout =
      cutoutLower - _tangent(cutoutLowerAngle, clockwise: true) * lowerHandle;

  return Path()
    ..moveTo(cutoutUpper.dx, cutoutUpper.dy)
    ..cubicTo(
      upperFromCutout.dx,
      upperFromCutout.dy,
      upperIntoAvatar.dx,
      upperIntoAvatar.dy,
      avatarUpper.dx,
      avatarUpper.dy,
    )
    // The long way round the avatar, back to the other side of the notch.
    ..arcTo(
      avatarRect,
      avatarUpperAngle,
      _arcSweep(
        avatarUpperAngle,
        avatarLowerAngle,
        clockwise: false,
        largeArc: true,
      ),
      false,
    )
    ..cubicTo(
      lowerFromAvatar.dx,
      lowerFromAvatar.dy,
      lowerIntoCutout.dx,
      lowerIntoCutout.dy,
      cutoutLower.dx,
      cutoutLower.dy,
    )
    // Back across the notch itself, which bites into the avatar.
    ..arcTo(
      Rect.fromCircle(center: cutoutCenter, radius: cutoutRadius),
      cutoutLowerAngle,
      _arcSweep(
        cutoutLowerAngle,
        cutoutUpperAngle,
        clockwise: true,
        largeArc: false,
      ),
      false,
    )
    ..close();
}

/// Clips an avatar to the rounded badge-notch path described by [geometry].
class AvatarBadgeMaskClipper extends CustomClipper<Path> {
  const AvatarBadgeMaskClipper({required this.geometry});

  final AvatarBadgeMaskGeometry geometry;

  @override
  Path getClip(Size size) =>
      avatarBadgeMaskPath(size: size.shortestSide, geometry: geometry);

  @override
  bool shouldReclip(covariant AvatarBadgeMaskClipper oldClipper) =>
      oldClipper.geometry != geometry;
}

/// A square [avatar] with [badge] seated in a notch cut out of its bottom-right
/// corner. With no [badge] the avatar is left whole — there is nothing to make
/// room for.
class MaskedAvatarBadge extends StatelessWidget {
  const MaskedAvatarBadge({
    super.key,
    required this.size,
    required this.avatar,
    this.geometry = AvatarBadgeMaskGeometry.badge,
    this.badge,
  });

  final double size;
  final Widget avatar;
  final AvatarBadgeMaskGeometry geometry;
  final Widget? badge;

  @override
  Widget build(BuildContext context) {
    if (badge == null) {
      return SizedBox.square(dimension: size, child: avatar);
    }

    final badgeSize = geometry.badgeSize(size);
    final center = geometry.cutoutCenter(size);

    return SizedBox.square(
      dimension: size,
      child: Stack(
        // The notch reaches past the avatar box, so the badge does too.
        clipBehavior: Clip.none,
        children: [
          ClipPath(
            clipper: AvatarBadgeMaskClipper(geometry: geometry),
            child: SizedBox.square(dimension: size, child: avatar),
          ),
          Positioned(
            left: center.dx - badgeSize / 2,
            top: center.dy - badgeSize / 2,
            width: badgeSize,
            height: badgeSize,
            child: badge!,
          ),
        ],
      ),
    );
  }
}
