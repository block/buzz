import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/shared/me/me_assistants.dart';
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
      var forms = 0;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(bridge, (
        call,
      ) async {
        if (++forms > 1) return null;
        return {'name': 'Mobile Solo', 'assistant': agent.public};
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
            meGroupsProvider.overrideWith(
              (ref) => Stream.value(const MeGroups()),
            ),
            meAssistantsProvider.overrideWith((ref) async => []),
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
      tester
          .widget<FrostedAppBar>(find.byType(FrostedAppBar))
          .nativeActions!
          .firstWhere((action) => action.label == 'New chat')
          .onPressed!();
      await tester.pumpAndSettle();
      final command = relay.submitted.single;
      expect(command.kind, 9007);
      expect(command.pubkey, owner.public);
      expect(command.tags, contains(equals(['t', 'buzz.solo'])));
      expect(command.tags, contains(equals(['channel_type', 'stream'])));
      expect(command.tags, contains(equals(['visibility', 'private'])));
      expect(command.tags.where((tag) => tag.first == 'P'), isEmpty);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );
}
