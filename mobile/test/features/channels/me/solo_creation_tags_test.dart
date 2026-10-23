import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _EmptyChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

class _Relay extends RelaySessionNotifier {
  final NostrEvent profile;
  final submitted = <NostrEvent>[];
  _Relay(this.profile);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [profile];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    submitted.add(event);
    // Stop at the transport boundary: this test covers the real creation
    // command, not relay acceptance or the subsequent membership writes.
    throw StateError('Captured creation command');
  }
}

void main() {
  testWidgets(
    'New Solo chat submits its discovery label through the production action',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final owner = nostr.Keys.generate(), agent = nostr.Keys.generate();
      final digest = SHA256Digest()
          .process(
            Uint8List.fromList(
              utf8.encode('nostr:agent-auth:${agent.public}:'),
            ),
          )
          .map((v) => v.toRadixString(16).padLeft(2, '0'))
          .join();
      final profile = NostrEvent.fromJson(
        nostr.Event.from(
          kind: 0,
          content: jsonEncode({'display_name': 'Owned agent'}),
          secretKey: agent.secret,
          tags: [
            [
              'auth',
              owner.public,
              '',
              nostr.Schnorr.sign(secretKey: owner.secret, message: digest),
            ],
          ],
        ).toMap(),
      );
      final relay = _Relay(profile);
      final config = RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: owner.nsec,
      );
      final sync = MeSync(relay, config, prefs: prefs);
      const bridge = MethodChannel('buzz/profile_text_editor');
      // Let native chrome mount while the real verification isolate runs.
      const platformViews = MethodChannel('flutter/platform_views');
      final nativeViews = <MethodChannel>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        platformViews,
        (call) async {
          if (call.method == 'create') {
            final args = call.arguments as Map<Object?, Object?>;
            final channel = MethodChannel('${args['viewType']}/${args['id']}');
            nativeViews.add(channel);
            tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
              channel,
              (_) async => null,
            );
          }
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          platformViews,
          null,
        ),
      );
      addTearDown(() {
        for (final channel in nativeViews) {
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            channel,
            null,
          );
        }
      });
      final directory = Completer<List<AgentDirectoryEntry>>();
      final draft = Completer<Map<String, String>>();
      String? presentationId;
      Map<Object?, Object?>? agentUpdate;
      var forms = 0;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(bridge, (
        call,
      ) async {
        if (call.method == 'updateChatAssistants') {
          agentUpdate = call.arguments as Map<Object?, Object?>;
          return null;
        }
        if (++forms > 1) return null;
        presentationId = (call.arguments as Map)['presentationId'] as String;
        return draft.future;
      });
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          bridge,
          null,
        ),
      );
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            meSyncProvider.overrideWithValue(sync),
            relaySessionProvider.overrideWith(() => relay),
            meGroupsProvider.overrideWith(
              (ref) => Stream.value(const MeGroups()),
            ),
            meAssistantsProvider.overrideWith((ref) => directory.future),
            channelsProvider.overrideWith(_EmptyChannels.new),
            channelActionsProvider.overrideWith(
              (ref) => ChannelActions(
                ref: ref,
                session: relay,
                signedEventRelay: SignedEventRelay(
                  session: relay,
                  nsec: owner.nsec,
                ),
                currentPubkey: owner.public,
              ),
            ),
          ],
          child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
        ),
      );
      await tester.pumpAndSettle();
      final start = tester
          .widget<FrostedAppBar>(find.byType(FrostedAppBar))
          .nativeActions!
          .firstWhere((action) => action.label == 'New chat')
          .onPressed!;
      // Ownership verification runs in a real isolate; pumpAndSettle alone
      // only drains the widget test's fake clock, not that computation.
      start();
      await tester.pump();
      expect(forms, 1, reason: 'Compose opens even while agents are loading');
      directory.complete([
        AgentDirectoryEntry(pubkey: agent.public, displayName: 'Owned agent'),
      ]);
      await tester.pumpAndSettle();
      expect(agentUpdate?['presentationId'], presentationId);
      expect((agentUpdate?['assistants'] as List).single['id'], agent.public);
      draft.complete({'name': 'Mobile Solo', 'assistant': agent.public});
      for (
        var attempts = 0;
        attempts < 100 && relay.submitted.isEmpty;
        attempts++
      ) {
        await tester.pumpAndSettle();
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
      }
      await tester.pumpAndSettle();
      final command = relay.submitted.single;
      expect(command.kind, 9007);
      expect(command.pubkey, owner.public);
      expect(command.tags, contains(equals(['t', 'buzz.solo'])));
      expect(command.tags, contains(equals(['channel_type', 'stream'])));
      expect(command.tags, contains(equals(['visibility', 'private'])));
      expect(command.tags.where((tag) => tag.first == 'P'), isEmpty);
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );
}
