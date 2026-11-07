part of '../channel_detail_page_test.dart';

void messageAuthorVisibilityTests() {
  for (final dark in [false, true]) {
    for (final kind in [
      'dm',
      'solo',
      'group',
      'channel',
      'unknown',
      'outsider',
    ]) {
      testWidgets('message names: $kind dark=$dark', (tester) async {
        final participants = kind == 'unknown'
            ? <String>[]
            : ['self', 'alice', if (kind == 'group') 'bob'];
        final channel = Channel(
          id: _channelId,
          name: 'Conversation',
          channelType: kind == 'channel' || kind == 'solo' ? 'stream' : 'dm',
          labels: kind == 'solo' ? const ['buzz.solo'] : const [],
          visibility: 'private',
          description: '',
          createdBy: 'self',
          createdAt: DateTime(2025),
          memberCount: participants.length,
          participantPubkeys: participants,
          isMember: true,
        );
        await tester.pumpWidget(
          _buildTestable(
            channel: channel,
            personalChat: kind == 'solo',
            darkTheme: dark,
            signingPubkey: 'self',
            members: [
              for (final pubkey in participants)
                ChannelMember(
                  pubkey: pubkey,
                  role: 'member',
                  joinedAt: DateTime(2025),
                ),
            ],
            users: const {
              'self': UserProfile(pubkey: 'self', displayName: 'Me'),
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
            },
            messages: [
              _textMsg(
                id: 'incoming',
                pubkey: kind == 'outsider' ? 'bob' : 'alice',
                content: 'Hello',
              ),
              _textMsg(
                id: 'outgoing',
                pubkey: 'self',
                content: 'Hi',
                createdAt: 1100,
              ),
            ],
          ),
        );
        await tester.pumpAndSettle();
        final name = find.byKey(const ValueKey('message-author-incoming'));
        final hidesName = kind == 'dm' || kind == 'solo';
        expect(name, hidesName ? findsNothing : findsOneWidget);
        expect(
          find.byKey(const ValueKey('message-timestamp-incoming')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('message-timestamp-outgoing')),
          findsOneWidget,
        );
        final ownName = find.byKey(const ValueKey('message-author-outgoing'));
        final hidesOwnName = hidesName || kind == 'outsider';
        expect(ownName, hidesOwnName ? findsNothing : findsOneWidget);
        if (!hidesOwnName) {
          expect(tester.widget<Text>(ownName).data, 'You');
        }
        if (!hidesName) {
          final context = tester.element(name);
          expect(
            tester.widget<Text>(name).style?.color,
            context.tertiaryTextColor,
          );
        }
        expect(tester.takeException(), isNull);
      });
    }
  }

  testWidgets('a confirmed remote agent says Remote between name and time', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final channel = Channel(
      id: _channelId,
      name: 'Agents',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: 'self',
      createdAt: DateTime(2025),
      memberCount: 3,
      participantPubkeys: const ['self', 'cloud', 'device'],
      isMember: true,
    );
    await tester.pumpWidget(
      _buildTestable(
        channel: channel,
        signingPubkey: 'self',
        remoteAgents: const {'cloud'},
        users: const {
          'cloud': UserProfile(
            pubkey: 'cloud',
            displayName: 'Scout',
            ownerPubkey: 'self',
          ),
          'device': UserProfile(
            pubkey: 'device',
            displayName: 'Helper',
            ownerPubkey: 'self',
          ),
        },
        messages: [
          _textMsg(id: 'remote', pubkey: 'cloud', content: 'On it'),
          _textMsg(
            id: 'local',
            pubkey: 'device',
            content: 'Me too',
            createdAt: 1100,
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();
    final remote = find.text('· Remote');
    expect(remote, findsOneWidget);
    final name = tester.getRect(
      find.byKey(const ValueKey('message-author-remote')),
    );
    final time = tester.getRect(
      find.byKey(const ValueKey('message-timestamp-remote')),
    );
    final label = tester.getRect(remote);
    expect(label.left, greaterThan(name.right));
    expect(label.right, lessThan(time.left));
    // Screen readers hear "Remote", not the separator.
    final spoken = tester.getSemantics(remote).label;
    expect(spoken, contains('Remote'));
    expect(spoken, isNot(contains('·')));
    await tester.longPress(remote);
    await tester.pumpAndSettle();
    expect(find.text('Runs remotely on Builderlab'), findsOneWidget);
    semantics.dispose();
  });
}
