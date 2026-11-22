import 'dart:async';
import 'dart:ui' as ui;

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
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

class _Profiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {
    'bob': UserProfile(pubkey: 'bob', avatarUrl: 'https://example.org/bob.png'),
  };
}

class _TwoProfiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {
    'bob': UserProfile(
      pubkey: 'bob',
      avatarUrl: 'https://example.org/swap-bob.png',
    ),
    'alice': UserProfile(
      pubkey: 'alice',
      avatarUrl: 'https://example.org/swap-alice.png',
    ),
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

  testWidgets(
    'a chip that changes identity drops the previous portrait while the next loads',
    (tester) async {
      final requests = <String>[];
      final responses = <String, Completer<http.Response>>{};
      final pubkey = ValueNotifier('bob');
      final recorder = ui.PictureRecorder();
      Canvas(recorder).drawColor(Colors.red, BlendMode.src);
      final recorded = recorder.endRecording();
      final pixel = recorded.toImageSync(1, 1);
      recorded.dispose();
      addTearDown(pixel.dispose);

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            myPubkeyProvider.overrideWithValue('viewer'),
            userCacheProvider.overrideWith(_TwoProfiles.new),
            mediaGetAuthServiceProvider.overrideWithValue(
              MediaGetAuthService(
                baseUrl: 'https://relay.example.org',
                nsec: null,
              ),
            ),
            mediaDiskCacheProvider.overrideWithValue(null),
            mediaHttpClientProvider.overrideWithValue(
              MockClient((request) {
                final url = request.url.toString();
                requests.add(url);
                return responses
                    .putIfAbsent(url, Completer<http.Response>.new)
                    .future;
              }),
            ),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: ValueListenableBuilder(
                valueListenable: pubkey,
                builder: (context, key, _) =>
                    MessageMentionPill(label: key, pubkey: key, isAgent: false),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      await tester.pump();
      expect(requests, ['https://example.org/swap-bob.png']);
      final image = tester.widget<Image>(find.byType(Image));
      image.image
          .resolve(
            createLocalImageConfiguration(tester.element(find.byType(Image))),
          )
          .completer!
          // Completes Bob's load without decoding bytes.
          // ignore: invalid_use_of_protected_member
          .setImage(ImageInfo(image: pixel.clone()));
      await tester.pump();
      final bob = tester.widget<RawImage>(find.byType(RawImage)).image;
      expect(bob, isNotNull);

      pubkey.value = 'alice';
      await tester.pump();
      expect(requests.last, 'https://example.org/swap-alice.png');
      final loading = tester.widget<RawImage>(find.byType(RawImage)).image;
      expect(loading?.isCloneOf(bob!), isNot(isTrue));

      for (final response in responses.values) {
        if (!response.isCompleted) response.complete(http.Response('', 404));
      }
      await tester.pumpAndSettle();
    },
  );
}
