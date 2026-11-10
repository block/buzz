import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
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
import 'package:buzz/shared/crypto/nip44.dart';
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
  _Relay(this.profile, this.existing);
  final List<NostrEvent> existing;
  var groups = <NostrEvent>[];

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [profile];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [
    ...existing,
    ...groups,
  ].where((e) => filter.kinds.contains(e.kind)).toList();

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
        const mediaChannel = MethodChannel('buzz/media_upload');
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          mediaChannel,
          (call) async => call.method == 'clipboardHasImage' ? false : null,
        );
        addTearDown(
          () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            mediaChannel,
            null,
          ),
        );
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
        // A recovered creation keeps its saved agent even when discovery no
        // longer lists it.
        directory.complete([
          if (!recoverExisting)
            AgentDirectoryEntry(
              pubkey: agent.public,
              displayName: 'Owned agent',
            ),
        ]);
        var agentAdded = false;
        final bridgeCalls = <MethodCall>[];
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(bridge, (
          call,
        ) async {
          bridgeCalls.add(call);
          return null;
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
              savedPrefsProvider.overrideWithValue(prefs),
              defaultAgentProvider.overrideWith(
                (ref) => Stream.value(agent.public),
              ),
              agentDirectoryProvider.overrideWith((ref) async => []),
              agentOwnersProvider.overrideWith((ref) async => {}),
              customEmojiListProvider.overrideWithValue([]),
              meSyncProvider.overrideWithValue(sync),
              relaySessionProvider.overrideWith(() => relay),
              meGroupsProvider.overrideWith(
                (ref) => Stream.value(const MeGroups()),
              ),
              meAssistantsProvider.overrideWith((ref) => directory.future),
              channelsProvider.overrideWith(_EmptyChannels.new),
              channelMembersProvider(channelId).overrideWith(
                (ref) async => [
                  if (agentAdded)
                    ChannelMember(
                      pubkey: agent.public,
                      role: 'bot',
                      joinedAt: DateTime(2026),
                    ),
                ],
              ),
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
        expect(bridgeCalls, isEmpty, reason: 'No native New chat sheet');
        if (recoverExisting) {
          expect(find.byType(ComposeBar), findsNothing);
          expect(
            tester
                .widget<FrostedAppBar>(find.byType(FrostedAppBar).last)
                .nativeActions!
                .last
                .onPressed,
            isNull,
            reason: 'The saved agent stays locked',
          );
          await tester.tap(find.text('Retry'));
        } else {
          final composer = tester.widget<ComposeBar>(find.byType(ComposeBar));
          unawaited(
            composer.onSend('Mobile Solo', const []).catchError((_) {}),
          );
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
          final pending = MeChatCreation.pending(sync)!;
          expect(pending.id, channelId);
          expect(pending.name, 'Mobile Solo');
          expect(
            pending.initialMessage,
            isNull,
            reason: 'A named creation from an older build finishes as saved',
          );
          expect(command.tags, contains(equals(['p', agent.public])));

          // A conflicting pending Solo edit blocks placement. Retry offers the
          // recovery choice and keeps the saved creation for the next Retry.
          agentAdded = true;
          relay.submitted.clear();
          MeGroups organization(String group) => MeGroups(
            groups: [
              for (final id in ['original', 'phone', 'desktop'])
                {'id': id, 'name': id, 'defaultTemplateId': ''},
            ],
            assignments: {channelId: group},
            channels: const [channelId],
          );
          NostrEvent signed(String group, int timestamp) => NostrEvent.fromJson(
            nostr.Event.from(
              kind: 30078,
              content: nip44Encrypt(
                sync.key,
                organization(group).encode(sync.community),
              ),
              secretKey: sync.privateHex,
              createdAt: timestamp,
              tags: [
                ['d', MeGroups.coordinate(sync.community)],
                ['t', MeGroups.tag],
              ],
              verify: false,
            ).toMap(),
          );
          await prefs.setString(
            sync.pendingKey,
            jsonEncode({
              'base': signed('original', 1).toJson(),
              'event': signed('phone', 2).toJson(),
            }),
          );
          relay.groups = [signed('desktop', 3)];
          await tester.tap(find.text('Retry'));
          for (
            var attempts = 0;
            attempts < 100 &&
                find.text('Conflicting Solo changes').evaluate().isEmpty;
            attempts++
          ) {
            await tester.pumpAndSettle();
            await tester.runAsync(
              () => Future<void>.delayed(const Duration(milliseconds: 50)),
            );
          }
          expect(find.text('Conflicting Solo changes'), findsOneWidget);
          expect(relay.submitted, isEmpty);
          await tester.tap(find.text('Discard phone change'));
          await tester.pumpAndSettle();
          expect(prefs.getString(sync.pendingKey), isNull);
          expect(MeChatCreation.pending(sync)?.id, channelId);
          await tester.tap(find.text('Retry'));
          for (
            var attempts = 0;
            attempts < 100 && MeChatCreation.pending(sync) != null;
            attempts++
          ) {
            await tester.pumpAndSettle();
            await tester.runAsync(
              () => Future<void>.delayed(const Duration(milliseconds: 50)),
            );
          }
          await tester.pumpAndSettle();
          // Desktop already lists the chat, so Retry finishes the creation
          // without another organization write.
          expect(MeChatCreation.pending(sync), isNull);
          expect(relay.submitted, isEmpty);
          expect(find.text('Retry'), findsNothing);
        } else {
          final pending = MeChatCreation.pending(sync)!;
          expect(pending.name, 'Mobile Solo');
          expect(pending.initialMessage?.content, 'Mobile Solo');
          expect(pending.initialMessage?.pubkey, owner.public);
          expect(
            pending.initialMessage?.tags,
            contains(equals(['p', agent.public])),
          );
          expect(pending.initialMessage?.getTagValue('h'), pending.id);
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
