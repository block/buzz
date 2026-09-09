part of '../compose_bar_test.dart';

void _publicationTests() {
  for (final mode in [
    'deny',
    'missing',
    'error',
    'revoke',
    'removed',
    'allow',
  ]) {
    testWidgets('publication authorization $mode retains exact intent', (
      tester,
    ) async {
      final key = 'd' * 64;
      final signer = nostr.Keys.generate();
      var reads = 0;
      var sent = 0;
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(signer.nsec),
          currentPubkey: signer.public,
          relayAgents: [_testAgent(key)],
          channels: [
            _makeCurrentChannel(channelType: 'dm'),
            _makeSharedMemberChannel(),
          ],
          authorizationReader: (keys, viewer, destination, current) async {
            expect(keys, {key});
            expect(destination, 'channel-1');
            expect(current(), isTrue);
            reads++;
            if (mode == 'error') throw StateError('unavailable');
            if (mode == 'missing') return [];
            return [
              AgentDirectoryEntry(
                pubkey: key,
                ownerPubkey: viewer,
                respondTo: mode == 'deny' || (mode == 'revoke' && reads == 2)
                    ? 'nobody'
                    : 'anyone',
                channelIds: mode == 'removed' && reads == 2
                    ? []
                    : ['channel-1'],
              ),
            ];
          },
          onSend: (_, keys, {mediaTags = const []}) async {
            expect(keys, [key]);
            sent++;
          },
        ),
      );
      await _selectAndSendAgentMention(tester);
      expect(sent, mode == 'allow' ? 1 : 0);
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        mode == 'allow' ? '' : 'hello @Helper Bot',
      );
      if (mode != 'allow') {
        expect(
          find.textContaining('Could not authorize a mentioned agent'),
          findsOneWidget,
        );
      }
      expect(
        reads,
        const ['allow', 'revoke', 'removed'].contains(mode) ? 2 : 1,
      );
    });
  }

  for (final (boundary, switchCommunity) in [
    for (final boundary in ['reader', 'scan', 'scan error', 'prompt'])
      for (final change in [false, true]) (boundary, change),
  ]) {
    testWidgets(
      'authorization cancellation $boundary reports scope change only: $switchCommunity',
      (tester) async {
        final key = 'd' * 64;
        final signer = nostr.Keys.generate();
        final pending = Completer<List<AgentDirectoryEntry>>();
        final members = Completer<List<ChannelMember>>();
        var scanPending = false;
        var reads = 0;
        var sent = 0;
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(signer.nsec),
            currentPubkey: signer.public,
            relayConfig: () => _SwitchableRelayConfigNotifier(
              RelayConfig(baseUrl: 'https://relay.example', nsec: signer.nsec),
            ),
            membersLoader: () => boundary == 'prompt'
                ? Future.value(
                    scanPending
                        ? []
                        : [
                            ChannelMember(
                              pubkey: 'a' * 64,
                              displayName: 'Alice',
                              role: 'member',
                              joinedAt: DateTime(2025),
                            ),
                          ],
                  )
                : scanPending
                ? members.future
                : Future.value([]),
            relayAgents: [_testAgent(key)],
            channels: [
              _makeCurrentChannel(
                channelType: boundary == 'reader' ? 'dm' : 'stream',
              ),
              _makeSharedMemberChannel(),
            ],
            authorizationReader: (_, _, _, _) {
              reads++;
              return pending.future;
            },
            onSend: (_, _, {mediaTags = const []}) async {
              sent++;
            },
          ),
        );
        await _expandComposer(tester);
        await tester.enterText(find.byType(TextField), '@hel');
        await tester.pumpAndSettle();
        await tester.tap(find.text('Helper Bot'));
        await tester.pumpAndSettle();
        if (boundary == 'prompt') {
          await tester.enterText(find.byType(TextField), '@Helper Bot @ali');
          await tester.pumpAndSettle();
          await tester.tap(find.text('Alice'));
          await tester.pumpAndSettle();
        } else {
          await tester.enterText(find.byType(TextField), 'hello @Helper Bot');
        }
        final controller = tester
            .widget<TextField>(find.byType(TextField))
            .controller!;
        if (boundary != 'reader') {
          scanPending = true;
          ProviderScope.containerOf(
            tester.element(find.byType(ComposeBar)),
          ).invalidate(channelMembersProvider('channel-1'));
          await tester.pump();
        }
        await tester.tap(find.byIcon(LucideIcons.arrowUp));
        await tester.pump();
        expect(reads, boundary == 'reader' ? 1 : 0);
        if (boundary == 'prompt') {
          await tester.pump(const Duration(milliseconds: 300));
        }
        if (switchCommunity) {
          ProviderScope.containerOf(tester.element(find.byType(ComposeBar)))
              .read(relayConfigProvider.notifier)
              .update(baseUrl: 'https://other.example', nsec: signer.nsec);
          await tester.pump();
        } else {
          controller.text = 'new intent';
        }
        if (boundary == 'prompt') {
          await tester.tap(find.text('Invite'));
        }
        if (boundary == 'scan error') {
          members.completeError(StateError('scan unavailable'));
        } else if (boundary == 'scan') {
          members.complete([]);
        }
        pending.complete([
          AgentDirectoryEntry(
            pubkey: key,
            respondTo: 'anyone',
            channelIds: ['channel-1'],
          ),
        ]);
        await tester.pumpAndSettle();
        expect(sent, 0);
        expect(
          find.text('Message not sent: the community changed'),
          switchCommunity ? findsOneWidget : findsNothing,
        );
        expect(find.textContaining('Could not authorize'), findsNothing);
        expect(controller.text, switchCommunity ? '' : 'new intent');
      },
    );
  }
  testWidgets('revocation after upload retains agent draft and attachment', (
    tester,
  ) async {
    final signer = nostr.Keys.generate();
    final key = 'd' * 64;
    final response = Completer<http.Response>();
    var uploaded = false;
    var reads = 0;
    var sent = 0;
    final service = MediaUploadService(
      baseUrl: 'https://relay.example',
      nsec: signer.nsec,
      httpClient: http_testing.MockClient((_) {
        uploaded = true;
        return response.future;
      }),
      pickGalleryVideo: () async => null,
      pickGalleryImage: () async => null,
      pickGalleryImages: () async => [
        XFile.fromData(_pngBytes, name: 'tiny.png'),
      ],
    );
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: service,
        currentPubkey: signer.public,
        relayAgents: [_testAgent(key)],
        channels: [
          _makeCurrentChannel(channelType: 'dm'),
          _makeSharedMemberChannel(),
        ],
        authorizationReader: (_, viewer, _, _) async {
          reads++;
          return [
            AgentDirectoryEntry(
              pubkey: key,
              ownerPubkey: viewer,
              respondTo: reads == 1 ? 'anyone' : 'nobody',
              channelIds: ['channel-1'],
            ),
          ];
        },
        onSend: (_, _, {mediaTags = const []}) async {
          sent++;
        },
      ),
    );
    await _openSystemPhotoPicker(tester);
    await tester.pumpAndSettle();
    await _expandComposer(tester);
    await tester.enterText(find.byType(TextField), '@hel');
    await tester.pumpAndSettle();
    await tester.tap(find.text('Helper Bot'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'hello @Helper Bot');
    await tester.tap(find.byIcon(LucideIcons.arrowUp));
    await tester.runAsync(() async {
      for (var i = 0; i < 100 && !uploaded; i++) {
        await Future<void>.delayed(const Duration(milliseconds: 10));
      }
    });
    expect(uploaded, isTrue);
    response.complete(
      http.Response(
        jsonEncode({
          'url': 'https://relay.example/media/test.png',
          'sha256': '0' * 64,
          'size': 16,
          'type': 'image/png',
          'uploaded': 1,
        }),
        200,
      ),
    );
    await tester.runAsync(() async {
      await Future<void>.delayed(const Duration(milliseconds: 100));
    });
    await tester.pumpAndSettle();
    expect(reads, 2);
    expect(sent, 0);
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller!.text,
      'hello @Helper Bot',
    );
    expect(
      find.byKey(const ValueKey('composer-agent-mention-chip')),
      findsOneWidget,
    );
    expect(
      find.textContaining('Could not authorize a mentioned agent'),
      findsOneWidget,
    );
  });
}
