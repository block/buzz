part of '../channel_detail_page_test.dart';

void _soloSendTests() {
  for (final (visibility, own) in [
    ('private', true),
    ('private', false),
    ('open', true),
    ('open', false),
  ]) {
    testWidgets('$visibility Solo uses owner-aware sending: $own', (
      tester,
    ) async {
      final ordinary = _RecordingOrdinarySender(),
          solo = _RecordingOrdinarySender();
      final channel = Channel(
        id: _channelId,
        name: 'Labeled Solo',
        channelType: 'stream',
        visibility: visibility,
        labels: const ['buzz.solo'],
        description: '',
        createdBy: own ? 'self' : 'other',
        createdAt: DateTime(2026),
        memberCount: 2,
        isMember: true,
      );
      await tester.pumpWidget(
        _buildTestable(
          messages: [],
          channel: channel,
          signingPubkey: 'self',
          personalChat: own,
          organization: Stream.value(const MeGroups()),
          sender: ordinary,
          soloSender: solo,
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2026),
            ),
            ChannelMember(
              pubkey: 'agent',
              role: 'bot',
              joinedAt: DateTime(2026),
            ),
          ],
          home: ChannelDetailPage(channel: channel),
        ),
      );
      await tester.pumpAndSettle();
      await tester
          .widget<ComposeBar>(find.byType(ComposeBar))
          .onSend('Label message', own ? ['agent'] : []);
      expect(own ? solo.sent : ordinary.sent, ['Label message']);
      expect(own ? ordinary.sent : solo.sent, isEmpty);
    });
  }

  testWidgets('open Solo visitors retain ordinary read-only treatment', (
    tester,
  ) async {
    final ordinary = _RecordingOrdinarySender(),
        solo = _RecordingOrdinarySender();
    final channel = Channel(
      id: _channelId,
      name: 'Shared Solo',
      channelType: 'stream',
      visibility: 'open',
      labels: const ['buzz.solo'],
      description: '',
      createdBy: 'other',
      createdAt: DateTime(2026),
      memberCount: 1,
    );
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          _textMsg(
            id: 'shared-history',
            pubkey: 'other',
            content: 'Shared history',
          ),
        ],
        channel: channel,
        signingPubkey: 'self',
        organization: Stream.value(const MeGroups()),
        sender: ordinary,
        soloSender: solo,
        home: ChannelDetailPage(channel: channel),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Shared history'), findsOneWidget);
    expect(find.byType(ComposeBar), findsNothing);
    expect(find.byKey(const ValueKey('me-assistant-button')), findsNothing);
    expect(
      find.text(
        'This chat is read-only. Ask an owner or admin to participate.',
      ),
      findsOneWidget,
    );
    expect(ordinary.sent, isEmpty);
    expect(solo.sent, isEmpty);
  });

  testWidgets(
    'foreign Solo cannot force the personal sender through a route hint',
    (tester) async {
      final ordinary = _RecordingOrdinarySender(),
          solo = _RecordingOrdinarySender();
      final channel = Channel(
        id: _channelId,
        name: 'Shared Solo',
        channelType: 'stream',
        visibility: 'private',
        labels: const ['buzz.solo'],
        description: '',
        createdBy: 'other',
        createdAt: DateTime(2026),
        memberCount: 2,
        isMember: true,
      );
      await tester.pumpWidget(
        _buildTestable(
          messages: [],
          channel: channel,
          signingPubkey: 'self',
          personalChat: true,
          organization: Stream.value(const MeGroups(channels: [_channelId])),
          sender: ordinary,
          soloSender: solo,
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2026),
            ),
            ChannelMember(
              pubkey: 'agent',
              role: 'bot',
              joinedAt: DateTime(2026),
            ),
          ],
          home: ChannelDetailPage(channel: channel, personalChat: true),
        ),
      );
      await tester.pumpAndSettle();
      await expectLater(
        tester.widget<ComposeBar>(find.byType(ComposeBar)).onSend('Rejected', [
          'agent',
        ]),
        throwsA(
          isA<StateError>().having(
            (e) => e.message,
            'message',
            'Refresh Solo before sending to this chat.',
          ),
        ),
      );
      expect(solo.sent, isEmpty);
      expect(ordinary.sent, isEmpty);
    },
  );

  testWidgets('identified Solo sends through the Solo sender', (tester) async {
    final ordinary = _RecordingOrdinarySender(),
        solo = _RecordingOrdinarySender();
    await tester.pumpWidget(
      _buildTestable(
        messages: [],
        personalChat: true,
        sender: ordinary,
        soloSender: solo,
        members: [
          ChannelMember(
            pubkey: 'self',
            role: 'member',
            joinedAt: DateTime(2026),
          ),
          ChannelMember(pubkey: 'agent', role: 'bot', joinedAt: DateTime(2026)),
        ],
      ),
    );
    await tester.pumpAndSettle();
    await tester.widget<ComposeBar>(find.byType(ComposeBar)).onSend(
      'Solo message',
      ['agent'],
    );
    expect(solo.sent, ['Solo message']);
    expect(ordinary.sent, isEmpty);
  });
}
