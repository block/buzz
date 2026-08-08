import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/profile/profile_avatar.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/masked_avatar_badge.dart';

void main() {
  Widget harness({bool showPresence = true, String presence = 'online'}) {
    return ProviderScope(
      overrides: [
        profileProvider.overrideWith(_FakeProfileNotifier.new),
        presenceProvider.overrideWith(() => _FakePresenceNotifier(presence)),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Center(child: ProfileAvatar(showPresence: showPresence)),
      ),
    );
  }

  testWidgets('seats the presence dot in a notch instead of over the avatar', (
    tester,
  ) async {
    await tester.pumpWidget(harness());
    await tester.pumpAndSettle();

    final badge = tester.widget<MaskedAvatarBadge>(
      find.byType(MaskedAvatarBadge),
    );
    expect(badge.geometry, AvatarBadgeMaskGeometry.presenceDot);
    expect(
      tester.widget<ClipPath>(find.byType(ClipPath).last).clipper,
      isA<AvatarBadgeMaskClipper>(),
    );

    // The notch supplies the separation, so the dot carries no border ring.
    final dot = tester
        .widgetList<DecoratedBox>(find.byType(DecoratedBox))
        .map((box) => box.decoration)
        .whereType<BoxDecoration>()
        .firstWhere((decoration) => decoration.shape == BoxShape.circle);
    expect(dot.border, isNull);
  });

  Color dotColor(WidgetTester tester) => tester
      .widgetList<DecoratedBox>(find.byType(DecoratedBox))
      .map((box) => box.decoration)
      .whereType<BoxDecoration>()
      .firstWhere((decoration) => decoration.shape == BoxShape.circle)
      .color!;

  final theme = AppTheme.light();

  testWidgets('paints the dot with the presence color', (tester) async {
    await tester.pumpWidget(harness());
    await tester.pumpAndSettle();

    expect(dotColor(tester), theme.extension<AppColors>()!.success);
  });

  testWidgets('mutes the dot when offline', (tester) async {
    await tester.pumpWidget(harness(presence: 'offline'));
    await tester.pumpAndSettle();

    expect(dotColor(tester), theme.colorScheme.outline);
  });

  testWidgets('leaves the avatar unmasked when presence is hidden', (
    tester,
  ) async {
    await tester.pumpWidget(harness(showPresence: false));
    await tester.pumpAndSettle();

    final badge = tester.widget<MaskedAvatarBadge>(
      find.byType(MaskedAvatarBadge),
    );
    expect(badge.badge, isNull);
    expect(
      find.descendant(
        of: find.byType(MaskedAvatarBadge),
        matching: find.byType(ClipPath),
      ),
      findsNothing,
    );
  });
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'aabb', displayName: 'Test');
}

class _FakePresenceNotifier extends PresenceNotifier {
  _FakePresenceNotifier(this._presence);

  final String _presence;

  @override
  Future<String> build() async => _presence;
}
