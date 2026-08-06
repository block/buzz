import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/settings_profile_header.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/masked_avatar_badge.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('uses a bounded icon for an unresolved status shortcode', (
    tester,
  ) async {
    const missingShortcode = ':very_long_missing_custom_emoji:';
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          profileProvider.overrideWith(_FakeProfileNotifier.new),
          presenceProvider.overrideWith(() => _FakePresenceNotifier('online')),
          userStatusProvider.overrideWith(
            () => _FakeUserStatusNotifier(
              const UserStatus(
                text: 'Focusing',
                emoji: missingShortcode,
                updatedAt: 1,
              ),
            ),
          ),
          customEmojiListProvider.overrideWithValue(const []),
        ],
        child: const SettingsProfileHeader(),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byType(Hero), findsNothing);
    final badge = find.byType(MaskedAvatarBadge);
    expect(
      find.descendant(of: badge, matching: find.text(missingShortcode)),
      findsNothing,
    );
    expect(
      find.descendant(of: badge, matching: find.byIcon(LucideIcons.smile)),
      findsOneWidget,
    );
  });

  testWidgets(
    'keeps text-only status visible beside a changeable presence pill',
    (tester) async {
      final presenceNotifier = _FakePresenceNotifier('away');
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            profileProvider.overrideWith(_FakeProfileNotifier.new),
            presenceProvider.overrideWith(() => presenceNotifier),
            userStatusProvider.overrideWith(
              () => _FakeUserStatusNotifier(
                const UserStatus(text: 'Focusing', emoji: '', updatedAt: 1),
              ),
            ),
            customEmojiListProvider.overrideWithValue(const []),
          ],
          child: const SettingsProfileHeader(),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Focusing'), findsOneWidget);
      await tester.tap(find.text('Focusing'));
      await tester.pumpAndSettle();
      expect(find.text('Set a status'), findsOneWidget);
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller?.text,
        'Focusing',
      );
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('settings-presence-label')),
        findsOneWidget,
      );
      expect(find.text('Away'), findsOneWidget);
      expect(
        tester
            .widget<Text>(find.byKey(const ValueKey('settings-presence-label')))
            .style
            ?.fontSize,
        filterChipTextStyle.fontSize,
      );
      expect(
        tester
            .getSize(find.byKey(const ValueKey('settings-presence-target')))
            .height,
        48,
      );
      expect(
        tester
            .getSize(find.byKey(const ValueKey('settings-presence-pill')))
            .height,
        greaterThanOrEqualTo(31),
      );

      final presenceTarget = find.byKey(
        const ValueKey('settings-presence-target'),
      );
      final targetRect = tester.getRect(presenceTarget);
      await tester.tapAt(Offset(targetRect.center.dx, targetRect.bottom - 1));
      await tester.pump();

      final scale = tester.widget<ScaleTransition>(
        find.byKey(const ValueKey('activity-popover-scale')),
      );
      expect(scale.alignment, Alignment.topCenter);
      expect(
        find.byKey(const ValueKey('settings-presence-popover')),
        findsOneWidget,
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('settings-presence-online')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('settings-presence-away')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('settings-presence-offline')),
        findsOneWidget,
      );

      await tester.tap(find.byKey(const ValueKey('settings-presence-offline')));
      await tester.pumpAndSettle();
      expect(presenceNotifier.selected, ['offline']);
    },
  );
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'aabb', displayName: 'Test');
}

class _FakeUserStatusNotifier extends UserStatusNotifier {
  _FakeUserStatusNotifier(this._status);

  final UserStatus _status;

  @override
  Future<UserStatus?> build() async => _status;
}

class _FakePresenceNotifier extends PresenceNotifier {
  _FakePresenceNotifier(this._presence);

  final String _presence;
  final List<String> selected = [];

  @override
  Future<String> build() async => _presence;

  @override
  Future<void> setPresence(String status) async {
    selected.add(status);
  }
}
