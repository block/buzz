part of '../compose_bar_test.dart';

void agentPrefillTests() {
  testWidgets('prefills the agents the viewer last tagged, never as a draft', (
    tester,
  ) async {
    final viewer = 'f' * 64;
    final agent = 'a' * 64;
    final person = 'b' * 64;
    var agentInChannel = true;
    var agentName = 'Scout';
    var scope = <NostrEvent>[];
    List<String>? sent;
    NostrEvent message(String id, String author, int at, List<String> tags) =>
        NostrEvent(
          id: id,
          pubkey: author,
          createdAt: at,
          kind: EventKind.streamMessage,
          tags: [
            const ['h', 'channel-1'],
            for (final key in tags) ['p', key],
          ],
          content: '',
          sig: '',
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
      prefillScopeEvents: scope,
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
}
