part of '../compose_bar_test.dart';

void agentPrefillTests() {
  test('the latest own message follows the display order, not ids', () {
    final viewer = 'f' * 64;
    TimelineMessage message(String id, String agent) => TimelineMessage(
      id: id,
      pubkey: viewer,
      createdAt: 10,
      content: '',
      tags: [
        ['p', agent],
      ],
    );
    // A channel shows same-second messages in descending id order.
    final events = [message('b', 'a' * 64), message('a', 'c' * 64)];
    final latest = latestOwnMessageMentions(events, viewer: viewer);
    expect(latest?.messageId, 'a');
    expect(latest?.pubkeys, ['c' * 64]);
  });

  testWidgets('prefills the agents the viewer last tagged, never as a draft', (
    tester,
  ) async {
    final viewer = 'f' * 64;
    final agent = 'a' * 64;
    final person = 'b' * 64;
    var agentInChannel = true;
    var agentName = 'Scout';
    var scope = <TimelineMessage>[];
    List<String>? sent;
    TimelineMessage message(
      String id,
      String author,
      int at,
      List<String> tags,
    ) => TimelineMessage(
      id: id,
      pubkey: author,
      createdAt: at,
      content: '',
      tags: [
        const ['h', 'channel-1'],
        for (final key in tags) ['p', key],
      ],
    );
    Widget build() => _buildComposeBar(
      uploadService: _testUploadService(nostr.Keys.generate().nsec),
      currentPubkey: viewer,
      channels: [_makeCurrentChannel()],
      loadMembers: () async => [
        if (agentInChannel)
          ChannelMember(
            pubkey: agent,
            displayName: agentName,
            role: 'bot',
            joinedAt: DateTime(2025),
          ),
        ChannelMember(
          pubkey: person,
          displayName: 'Pat',
          role: 'member',
          joinedAt: DateTime(2025),
        ),
      ],
      prefillScopeMessages: scope,
      onSend: (_, mentions, {mediaTags = const []}) async => sent = mentions,
    );
    // The collapsed composer previews its trimmed text, or the hint.
    String text() => find.byType(TextField).evaluate().isNotEmpty
        ? tester.widget<TextField>(find.byType(TextField)).controller!.text
        : find.text('@$agentName').evaluate().isNotEmpty
        ? '@$agentName '
        : find.text('Message\u2026').evaluate().isNotEmpty
        ? ''
        : 'unexpected';
    ComposeDraft? savedDraft() =>
        ProviderScope.containerOf(tester.element(find.byType(ComposeBar)))
            .read(composeDraftsProvider.notifier)
            .draftFor(composeDraftKey('channel-1'));

    // The viewer's latest message tagged an agent and a person; someone
    // else's newer message does not count. Only the agent carries forward.
    scope = [
      message('1', viewer, 10, [person]),
      message('2', viewer, 20, [agent, person]),
      message('3', person, 30, [viewer]),
    ];
    await tester.pumpWidget(build());
    await tester.pumpAndSettle();
    expect(text(), '@Scout ');
    expect(savedDraft(), isNull);

    // Typing makes it the viewer's draft, and its agent sends.
    await tester.tap(find.text('@Scout'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '@Scout hi');
    await tester.pumpAndSettle();
    expect(savedDraft()?.text, '@Scout hi');
    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(sent, [agent]);
    expect(text(), isEmpty);

    // The sent message becomes the latest and prefills again.
    scope = [
      ...scope,
      message('4', viewer, 40, [agent]),
    ];
    await tester.pumpWidget(build());
    await tester.pumpAndSettle();
    expect(text(), '@Scout ');

    // A removed prefill stays removed while the composer stays shown, even
    // when the agent's name changes...
    await tester.enterText(find.byType(TextField), '');
    await tester.pumpAndSettle();
    agentName = 'Ranger';
    ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    ).invalidate(channelMembersProvider('channel-1'));
    await tester.pumpAndSettle();
    expect(text(), isEmpty);

    // ...and returns when it is shown again.
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpWidget(build());
    await tester.pumpAndSettle();
    expect(text(), '@Ranger ');

    // An agent no longer in the channel is not prefilled.
    await tester.pumpWidget(const SizedBox.shrink());
    agentInChannel = false;
    await tester.pumpWidget(build());
    await tester.pumpAndSettle();
    expect(text(), isEmpty);
  });

  testWidgets('takes back an untouched prefill once it is out of date', (
    tester,
  ) async {
    final viewer = 'f' * 64;
    final agent = 'a' * 64;
    final person = 'b' * 64;
    var agentInChannel = true;
    var scope = <TimelineMessage>[];
    TimelineMessage message(String id, int at, List<String> tags) =>
        TimelineMessage(
          id: id,
          pubkey: viewer,
          createdAt: at,
          content: '',
          tags: [
            const ['h', 'channel-1'],
            for (final key in tags) ['p', key],
          ],
        );
    Widget build() => _buildComposeBar(
      uploadService: _testUploadService(nostr.Keys.generate().nsec),
      currentPubkey: viewer,
      channels: [_makeCurrentChannel()],
      loadMembers: () async => [
        if (agentInChannel)
          ChannelMember(
            pubkey: agent,
            displayName: 'Scout',
            role: 'bot',
            joinedAt: DateTime(2025),
          ),
      ],
      prefillScopeMessages: scope,
      onSend: (_, _, {mediaTags = const []}) async {},
    );
    bool shows(String text) => find.text(text).evaluate().isNotEmpty;
    void dropAgent() {
      agentInChannel = false;
      ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      ).invalidate(channelMembersProvider('channel-1'));
    }

    // A newer message (sent elsewhere) that tags no agent clears the prefill.
    scope = [
      message('1', 10, [agent]),
    ];
    await tester.pumpWidget(build());
    await tester.pumpAndSettle();
    expect(shows('@Scout'), isTrue);
    scope = [
      ...scope,
      message('2', 20, [person]),
    ];
    await tester.pumpWidget(build());
    await tester.pumpAndSettle();
    expect(shows('@Scout'), isFalse);
    expect(shows('Message…'), isTrue);

    // So does the last tagged agent leaving the channel.
    scope = [
      ...scope,
      message('3', 30, [agent]),
    ];
    await tester.pumpWidget(build());
    await tester.pumpAndSettle();
    expect(shows('@Scout'), isTrue);
    dropAgent();
    await tester.pumpAndSettle();
    expect(shows('@Scout'), isFalse);
    expect(shows('Message…'), isTrue);

    // Text the viewer wrote is theirs, and stays.
    agentInChannel = true;
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpWidget(build());
    await tester.pumpAndSettle();
    await tester.tap(find.text('@Scout'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '@Scout hi');
    await tester.pumpAndSettle();
    dropAgent();
    await tester.pumpAndSettle();
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller!.text,
      '@Scout hi',
    );
  });
}
