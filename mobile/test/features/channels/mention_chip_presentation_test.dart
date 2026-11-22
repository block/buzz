import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/message_mention_pill.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _Profiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {
    'bob': UserProfile(pubkey: 'bob', avatarUrl: 'https://example.org/bob.png'),
  };
}

void main() {
  testWidgets(
    'mentions follow identity, bubble, and appearance while staying tappable',
    (tester) async {
      final profiles = userCacheProvider.overrideWith(_Profiles.new);
      final container = ProviderContainer(
        overrides: [myPubkeyProvider.overrideWithValue('alice'), profiles],
      );
      addTearDown(container.dispose);
      String? tapped;
      const names = {'alice': 'Alice', 'bob': 'Bob'};
      for (final brightness in Brightness.values) {
        for (final own in [false, true]) {
          final theme = mobileAppearanceTheme(brightness, '#c0a2f1');
          final chat = theme.extension<ChatColors>()!;
          await tester.pumpWidget(
            UncontrolledProviderScope(
              container: container,
              child: MaterialApp(
                theme: theme,
                home: Scaffold(
                  body: MediaQuery(
                    data: const MediaQueryData(
                      size: Size(800, 600),
                      textScaler: TextScaler.linear(2),
                    ),
                    child: AppMarkdownTheme(
                      child: SizedBox(
                        width: 320,
                        child: MessageContent(
                          content: 'Hello @Alice and @Bob',
                          mentionNames: names,
                          bubble: true,
                          ownMessage: own,
                          onMentionTap: (key) => tapped = key,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          );
          await tester.pumpAndSettle();
          Color fill(String label) {
            final pill = find.ancestor(
              of: find.text(label),
              matching: find.byType(MessageMentionPill),
            );
            final box = tester.widget<Container>(
              find.descendant(of: pill, matching: find.byType(Container)).first,
            );
            return (box.decoration! as BoxDecoration).color!;
          }

          expect(
            fill('Alice'),
            own ? chatCodeTint(chat.foreground, .18) : chat.outgoing,
          );
          expect(
            fill('Bob'),
            own
                ? chatCodeTint(chat.foreground, .18)
                : (brightness == Brightness.dark
                      ? Colors.white.withValues(alpha: .16)
                      : Colors.black.withValues(alpha: .10)),
          );
          final portrait = tester.widget<AvatarImage>(find.byType(AvatarImage));
          expect(portrait.imageUrl, 'https://example.org/bob.png');
          expect(portrait.isAgent, isFalse);
          tapped = null;
          await tester.tap(find.text('Bob'));
          expect(tapped, 'bob');
          expect(tester.takeException(), isNull);

          // Switching identity updates already-parsed spans, without new content.
          container.updateOverrides([
            profiles,
            myPubkeyProvider.overrideWithValue('bob'),
          ]);
          await tester.pump();
          if (!own) {
            expect(fill('Bob'), chat.outgoing);
            expect(fill('Alice'), isNot(chat.outgoing));
          }
          container.updateOverrides([
            profiles,
            myPubkeyProvider.overrideWithValue('alice'),
          ]);
          await tester.pump();
        }
      }
    },
  );
}
