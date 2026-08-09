import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/masked_avatar_badge.dart';

void main() {
  group('avatarBadgeMaskPath', () {
    const size = 128.0;
    const geometry = AvatarBadgeMaskGeometry.badge;

    test('keeps the avatar but cuts the notch out of it', () {
      final path = avatarBadgeMaskPath(size: size, geometry: geometry);

      expect(path.contains(const Offset(size / 2, size / 2)), isTrue);
      expect(path.contains(geometry.cutoutCenter(size)), isFalse);
      // The far edge is untouched — the notch only bites the bottom-right.
      expect(path.contains(const Offset(4, size / 2)), isTrue);
    });

    test('stays inside the avatar circle on the unnotched sides', () {
      // Checked with contains() rather than getBounds(), which reports
      // control-point bounds and so overshoots the near-full-circle arc.
      final path = avatarBadgeMaskPath(size: size, geometry: geometry);

      expect(path.contains(const Offset(-1, size / 2)), isFalse);
      expect(path.contains(const Offset(size / 2, -1)), isFalse);
      expect(path.contains(const Offset(2, 2)), isFalse);
    });

    test('fillets the notch rather than subtracting a bare circle', () {
      // A plain difference meets the cutout at two cusps. The fillets round
      // those off, which necessarily removes extra material just outside the
      // cutout circle — so the masked path must be a strict subset there.
      final masked = avatarBadgeMaskPath(size: size, geometry: geometry);
      final cutoutCenter = geometry.cutoutCenter(size);
      final cutoutRadius = geometry.cutoutRadius(size);
      final bare = Path.combine(
        PathOperation.difference,
        Path()..addOval(
          Rect.fromCircle(
            center: const Offset(size / 2, size / 2),
            radius: size / 2,
          ),
        ),
        Path()..addOval(
          Rect.fromCircle(center: cutoutCenter, radius: cutoutRadius),
        ),
      );

      // Sample the ring just outside the cutout, where the fillets live.
      var filleted = 0;
      for (var step = 0; step < 360; step++) {
        final angle = step * math.pi / 180;
        final point =
            cutoutCenter +
            Offset(math.cos(angle), math.sin(angle)) * (cutoutRadius + 2);
        if (bare.contains(point) && !masked.contains(point)) filleted++;
        // The fillet only adds material to the cut, never puts it back.
        expect(
          masked.contains(point) && !bare.contains(point),
          isFalse,
          reason: 'mask should not extend beyond the plain difference',
        );
      }
      expect(filleted, greaterThan(0));
    });

    test('falls back to a plain circle when the notch misses the avatar', () {
      // A notch tucked at the centre never crosses the avatar's edge, so there
      // is no crossing to fillet and nothing should be cut.
      const detached = AvatarBadgeMaskGeometry(
        cutoutCenterRatio: 0.5,
        cutoutRadiusRatio: 0.05,
        badgeSizeRatio: 0.08,
        curve: AvatarBadgeCurve.standard,
      );
      final path = avatarBadgeMaskPath(size: size, geometry: detached);

      expect(path.contains(detached.cutoutCenter(size)), isTrue);
      expect(path.getBounds().size, const Size(size, size));
    });

    test(
      'scales geometry with the avatar, matching desktop at its own size',
      () {
        // Desktop's sidebar avatar is 32px with the cutout at (28, 28) r 7.5.
        const presence = AvatarBadgeMaskGeometry.presenceDot;

        expect(presence.cutoutCenter(32), const Offset(28, 28));
        expect(presence.cutoutRadius(32), 7.5);
        expect(presence.badgeSize(32), 14);
        // Twice the avatar, twice the notch.
        expect(presence.cutoutCenter(64), const Offset(56, 56));
        expect(presence.cutoutRadius(64), 15);
      },
    );
  });

  group('MaskedAvatarBadge', () {
    Widget harness({Widget? badge}) => MaterialApp(
      theme: AppTheme.light(),
      home: Center(
        child: MaskedAvatarBadge(
          size: 128,
          geometry: AvatarBadgeMaskGeometry.badge,
          avatar: const ColoredBox(color: Color(0xFF123456)),
          badge: badge,
        ),
      ),
    );

    testWidgets('masks the avatar and centres the badge in the notch', (
      tester,
    ) async {
      await tester.pumpWidget(
        harness(badge: const ColoredBox(color: Color(0xFFABCDEF))),
      );

      final clip = tester.widget<ClipPath>(find.byType(ClipPath));
      expect(clip.clipper, isA<AvatarBadgeMaskClipper>());

      const geometry = AvatarBadgeMaskGeometry.badge;
      final frame = tester.getRect(find.byType(MaskedAvatarBadge));
      final badgeRect = tester.getRect(find.byType(ColoredBox).last);

      expect(badgeRect.width, closeTo(geometry.badgeSize(128), 0.01));
      expect(
        badgeRect.center - frame.topLeft,
        within(distance: 0.01, from: geometry.cutoutCenter(128)),
      );
    });

    testWidgets('leaves the avatar whole when there is no badge', (
      tester,
    ) async {
      await tester.pumpWidget(harness());

      expect(find.byType(ClipPath), findsNothing);
      expect(
        tester.getSize(find.byType(MaskedAvatarBadge)),
        const Size(128, 128),
      );
    });
  });
}
