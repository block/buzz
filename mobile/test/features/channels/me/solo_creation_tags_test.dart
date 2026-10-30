import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:flutter/foundation.dart';
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
  _Relay(this.profile, this.existing);
  final List<NostrEvent> existing;

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [profile];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => existing.where((e) => filter.kinds.contains(e.kind)).toList();

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
  for (final recoverExisting in [false, true]) {
    testWidgets(
      recoverExisting
          ? 'interrupted Solo recovery uses fresh membership before creating'
          : 'New Solo chat submits its discovery label through the production action',
      (tester) async {
        final flutterSheet = defaultTargetPlatform == TargetPlatform.android;
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
        const channelId = '10000000-0000-4000-8000-000000000001';
        final relay = _Relay(profile, [
          if (recoverExisting)
            for (final kind in [39000, 39002])
              NostrEvent.fromJson(
                nostr.Event.from(
                  kind: kind,
                  content: '',
                  secretKey: owner.secret,
                  tags: [
                    ['d', channelId],
                    if (kind == 39000) ['name', 'Mobile Solo'],
                    if (kind == 39002) ['p', owner.public],
                  ],
                ).toMap(),
              ),
        ]);
        final config = RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: owner.nsec,
        );
        final sync = MeSync(relay, config, prefs: prefs);
        if (recoverExisting) {
          await MeChatCreation(
            channelId,
            'Mobile Solo',
            agent.public,
          ).save(sync);
        }
        const bridge = MethodChannel('buzz/profile_text_editor');
        // Let native chrome mount while the real verification isolate runs.
        const platformViews = MethodChannel('flutter/platform_views');
        final nativeViews = <MethodChannel>[];
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          platformViews,
          (call) async {
            if (call.method == 'create') {
              final args = call.arguments as Map<Object?, Object?>;
              final channel = MethodChannel(
                '${args['viewType']}/${args['id']}',
              );
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
        if (flutterSheet) directory.complete([]);
        final draft = Completer<Map<String, String>>();
        String? presentationId;
        Map<Object?, Object?>? agentUpdate;
        Object? savedAssistant;
        var forms = 0;
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(bridge, (
          call,
        ) async {
          if (call.method == 'updateChatAssistants') {
            agentUpdate = call.arguments as Map<Object?, Object?>;
            return null;
          }
          if (++forms > 1) return null;
          final arguments = call.arguments as Map;
          presentationId = arguments['presentationId'] as String;
          savedAssistant = arguments['savedAssistant'];
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
              channelMembersProvider(channelId).overrideWith((ref) async => []),
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
        await tester.pumpAndSettle();
        if (recoverExisting) {
          await tester.tap(find.text('Resume'));
          await tester.pumpAndSettle();
        }
        if (flutterSheet) {
          final row = find.byKey(const ValueKey('new-session-assistant'));
          if (!recoverExisting) {
            expect(find.text('Choose an agent'), findsOneWidget);
            await tester.enterText(
              find.byKey(const ValueKey('new-session-name')),
              'Mobile Solo',
            );
            await tester.tap(find.text('Create chat'));
            await tester.pumpAndSettle();
            expect(
              find.text('Enter a name and choose an agent.'),
              findsOneWidget,
            );
            expect(relay.submitted, isEmpty);
            expect(MeChatCreation.pending(sync), isNull);
            await tester.pumpWidget(const SizedBox.shrink());
            await tester.pumpAndSettle();
            return;
          }
          final semantics = tester.ensureSemantics();
          await tester.pump();
          expect(find.text('Choose an agent'), findsNothing);
          expect(find.text(agent.public), findsOneWidget);
          expect(find.bySemanticsLabel(agent.public), findsOneWidget);
          expect(tester.widget<AppListRowRaw>(row).onTap, isNull);
          final container = ProviderScope.containerOf(tester.element(row));
          container
              .read(userCacheProvider.notifier)
              .put(
                UserProfile(pubkey: agent.public, displayName: 'Saved agent'),
              );
          await tester.pumpAndSettle();
          expect(find.text('Saved agent'), findsOneWidget);
          expect(find.bySemanticsLabel('Saved agent'), findsOneWidget);
          expect(tester.widget<AppListRowRaw>(row).onTap, isNull);
          semantics.dispose();
          await tester.tap(find.text('Retry'));
        } else {
          expect(
            forms,
            1,
            reason: 'Compose opens even while agents are loading',
          );
          if (recoverExisting) {
            expect(savedAssistant, {
              'id': agent.public,
              'name': agent.public,
              'avatarUrl': '',
            });
          }
          directory.complete([
            AgentDirectoryEntry(
              pubkey: agent.public,
              displayName: 'Owned agent',
            ),
          ]);
          await tester.pumpAndSettle();
          expect(agentUpdate?['presentationId'], presentationId);
          expect(
            (agentUpdate?['assistants'] as List).single['id'],
            agent.public,
          );
          draft.complete({'name': 'Mobile Solo', 'assistant': agent.public});
        }
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
        if (recoverExisting) {
          expect(relay.submitted.where((event) => event.kind == 9007), isEmpty);
          expect(
            command.kind,
            9000,
            reason: 'Recovery continues with member setup',
          );
          expect(MeChatCreation.pending(sync)?.id, channelId);
          expect(command.tags, contains(equals(['p', agent.public])));
        } else {
          expect(command.kind, 9007);
          expect(command.pubkey, owner.public);
          expect(command.tags, contains(equals(['t', 'buzz.solo'])));
          expect(command.tags, contains(equals(['channel_type', 'stream'])));
          expect(command.tags, contains(equals(['visibility', 'private'])));
          expect(command.tags.where((tag) => tag.first == 'P'), isEmpty);
        }
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pumpAndSettle();
      },
      variant: TargetPlatformVariant({
        TargetPlatform.iOS,
        TargetPlatform.android,
      }),
    );
  }
}
