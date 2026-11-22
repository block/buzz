import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/features/channels/dm_destination.dart';
import 'package:buzz/features/channels/dm_draft_page.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

Channel _dm(
  String id,
  List<String> participants, {
  bool archived = false,
  bool member = true,
}) => Channel(
  id: id,
  name: id,
  channelType: 'dm',
  visibility: 'private',
  description: '',
  memberCount: participants.length,
  createdBy: 'self',
  createdAt: DateTime(2025),
  isMember: member,
  archivedAt: archived ? DateTime(2026) : null,
  participantPubkeys: participants,
);

void main() {
  final peer = 'a' * 64;
  final other = 'b' * 64;

  test('finds only your listed DM with exactly those people', () {
    final pair = _dm('pair', ['self', peer]);
    final group = _dm('group', ['self', peer, other]);
    expect(existingDmWith([group, pair], 'self', [peer.toUpperCase()]), pair);
    expect(existingDmWith([group, pair], 'SELF', [other, peer]), group);
    expect(existingDmWith([group], 'self', [peer]), isNull);
    expect(
      existingDmWith(
        [
          _dm('old', ['self', peer], archived: true),
        ],
        'self',
        [peer],
      ),
      isNull,
    );
    expect(
      existingDmWith(
        [
          _dm('left', ['self', peer], member: false),
        ],
        'self',
        [peer],
      ),
      isNull,
    );
  });

  testWidgets('the first send creates the DM once and then shows it', (
    tester,
  ) async {
    final dm = _dm('dm-peer', ['self', peer]);
    final opened = <List<String>>[];
    final sender = _Sender()..failNext = true;
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          currentPubkeyProvider.overrideWith((ref) => 'self'),
          agentOwnersProvider.overrideWithValue(
            const AsyncValue.data(<String, String>{}),
          ),
          userCacheProvider.overrideWith(
            () => _UserCache({
              peer: UserProfile(pubkey: peer, displayName: 'Peer'),
            }),
          ),
          channelsProvider.overrideWith(() => _Channels([dm])),
          channelActionsProvider.overrideWith((ref) => _Actions(dm, opened)),
          sendMessageProvider.overrideWithValue(sender),
          savedPrefsProvider.overrideWithValue(prefs),
          meGroupsProvider.overrideWith((ref) => const Stream.empty()),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: DmDraftPage(pubkeys: [peer]),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.textContaining('starts when you send'), findsOneWidget);
    expect(opened, isEmpty);

    Future<void> send(String text) =>
        tester.widget<ComposeBar>(find.byType(ComposeBar)).onSend(text, []);

    // A failed send keeps the draft; its retry reuses the DM it opened.
    await expectLater(send('hi'), throwsException);
    await tester.pumpAndSettle();
    expect(find.byType(DmDraftPage), findsOneWidget);
    expect(find.byType(ChannelDetailPage), findsNothing);
    await send('hi');
    await tester.pumpAndSettle();

    expect(opened, [
      [peer],
    ]);
    expect(sender.sent, ['dm-peer', 'dm-peer']);
    expect(
      tester.widget<ChannelDetailPage>(find.byType(ChannelDetailPage)).channel,
      dm,
    );
  });
}

class _Sender implements SendMessage {
  final sent = <String>[];
  bool failNext = false;

  @override
  Future<void> call({
    required String channelId,
    required String content,
    String? parentEventId,
    String? rootEventId,
    List<String>? mentionPubkeys,
    Channel? channel,
    List<List<String>> mediaTags = const [],
  }) async {
    sent.add(channelId);
    if (failNext) {
      failNext = false;
      throw StateError('Offline');
    }
  }
}

class _UserCache extends UserCacheNotifier {
  _UserCache(this._initial);

  final Map<String, UserProfile> _initial;

  @override
  Map<String, UserProfile> build() => _initial;

  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}

class _Channels extends ChannelsNotifier {
  _Channels(this._channels);

  final List<Channel> _channels;

  @override
  Future<List<Channel>> build() async => _channels;
}

class _Actions implements ChannelActions {
  _Actions(this.dm, this.opened);

  final Channel dm;
  final List<List<String>> opened;

  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) async {
    opened.add(pubkeys);
    return dm;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
