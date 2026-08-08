import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/settings_profile_header.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/widgets/masked_avatar_badge.dart';
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
