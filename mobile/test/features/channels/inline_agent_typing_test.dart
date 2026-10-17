import 'package:buzz/features/channels/channel_identity_names_provider.dart';
import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/features/channels/inline_agent_typing.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const agent =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const profile = UserProfile(pubkey: agent, displayName: 'Test agent');

class Profiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => {agent: profile};
}

Widget surface({bool reduced = false, bool typing = true}) => ProviderScope(
  overrides: [
    userCacheProvider.overrideWith(Profiles.new),
    channelIdentityNamesProvider('chat').overrideWithValue(
      const IdentityNameSources(profiles: {agent: profile}).scope([agent]),
    ),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: MediaQuery(
      data: MediaQueryData(disableAnimations: reduced),
      child: Scaffold(
        body: InlineAgentTyping(
          channelId: 'chat',
          entries: [
            if (typing) const TypingEntry(pubkey: agent, expiresAtMs: 8000),
          ],
        ),
      ),
    ),
  ),
);
void main() {
  testWidgets(
    'message rail, author and dots replace status text without a timestamp',
    (tester) async {
      await tester.pumpWidget(surface());
      expect(find.text('Test agent'), findsOneWidget);
      expect(find.byType(AvatarImage), findsOneWidget);
      expect(find.textContaining('typing'), findsNothing);
      expect(find.byType(Text), findsNWidgets(2)); // Name and avatar initial.
      expect(find.byKey(const ValueKey('agent-typing-dot-2')), findsOneWidget);
      expect(find.byKey(const ValueKey('agent-typing-dot-0')), findsOneWidget);
      final dot = find.byKey(const ValueKey('agent-typing-dot-0'));
      final initial = tester
          .widget<Transform>(dot)
          .transform
          .getTranslation()
          .y;
      await tester.pump(const Duration(milliseconds: 240));
      expect(
        tester.widget<Transform>(dot).transform.getTranslation().y,
        lessThan(initial),
      );
      expect(
        tester.getTopLeft(find.text('Test agent')).dx,
        messageAvatarSize + messageAvatarContentGap + Grid.xs,
      );
      await tester.pumpWidget(surface(typing: false));
      expect(find.byType(AvatarImage), findsNothing);
      expect(find.text('Working…'), findsNothing);
    },
  );
  testWidgets('Reduce Motion leaves static dots and no active animation', (
    tester,
  ) async {
    await tester.pumpWidget(surface(reduced: true));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('agent-typing-dot-2')), findsOneWidget);
    expect(find.byType(ShaderMask), findsNothing);
    expect(tester.binding.hasScheduledFrame, isFalse);
  });
}
