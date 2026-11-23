import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

class _Relay extends RelaySessionNotifier {
  VoidCallback? beforeRead;
  int reads = 0;
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    reads++;
    beforeRead?.call();
    throw StateError('ownership lookup interrupted');
  }
}

void main() {
  for (final disconnect in [true, false]) {
    testWidgets(
      'chat accepted before ${disconnect ? "disconnect" : "preflight failure"} is durable',
      (tester) async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final config = RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data: '0' * 63 + '1',
          ),
        );
        final relay = _Relay();
        var sync = MeSync(relay, config, prefs: prefs);
        relay.beforeRead = () =>
            expectSync(MeChatCreation.pending(sync)?.name, 'Saved chat');
        late ProviderContainer container;
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              agentDirectoryProvider.overrideWith((ref) async => []),
              agentOwnersProvider.overrideWith((ref) async => {}),
              customEmojiListProvider.overrideWithValue([]),
              defaultAgentProvider.overrideWith((ref) => Stream.value('agent')),
              meSyncProvider.overrideWith((ref) => sync),
              meGroupsProvider.overrideWith(
                (ref) => Stream.value(const MeGroups()),
              ),
              meAssistantsProvider.overrideWith(
                (ref) async => const [
                  AgentDirectoryEntry(pubkey: 'agent', displayName: 'Agent'),
                ],
              ),
              channelsProvider.overrideWith(_Channels.new),
              channelActionsProvider.overrideWith(
                (ref) => ChannelActions(
                  ref: ref,
                  session: relay,
                  signedEventRelay: SignedEventRelay(
                    session: relay,
                    nsec: config.nsec,
                  ),
                  currentPubkey: sync.pubkey,
                ),
              ),
            ],
            child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
          ),
        );
        await tester.pumpAndSettle();
        container = ProviderScope.containerOf(
          tester.element(find.byType(MePage)),
        );
        tester
            .widget<FrostedAppBar>(find.byType(FrostedAppBar))
            .nativeActions!
            .firstWhere((a) => a.label == 'New chat')
            .onPressed!();
        await tester.pumpAndSettle();
        if (disconnect) {
          sync.disposed = true;
          sync = MeSync(relay, config, prefs: prefs, connected: false);
          container.invalidate(meSyncProvider);
          await tester.pumpAndSettle();
        }
        await expectLater(
          tester
              .widget<ComposeBar>(find.byType(ComposeBar))
              .onSend('Saved chat', const []),
          throwsException,
        );
        await tester.pumpAndSettle();
        final saved = MeChatCreation.pending(
          MeSync(relay, config, prefs: prefs),
        );
        expect(
          saved,
          isNotNull,
          reason: 'Accepted chat must survive before preflight succeeds',
        );
        expect(saved!.name, 'Saved chat');
        expect(saved.agent, 'agent');
        expect(saved.project, isNull);
        expect(
          prefs.getString(MeChatCreation.storageKey(sync)),
          isNot(contains('Saved chat')),
        );
        expect(relay.reads, disconnect ? 0 : 1);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );
  }
}
