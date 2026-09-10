part of '../compose_bar_test.dart';

void classificationTests() {
  for (final scenario in [
    'production consent profile',
    'production postack profile',
    'production invitation revoked',
    'production invitation draft',
    'production invitation capacity',
    'production final revoked',
    'production final capacity',
    'production final waiting revocation',
    'production final draft',
    'production postack scope',
    'production reference scope',
    'production mixed obsolete',
    'production mixed revoked',
    'production mixed obsolete media',
    'production mixed revoked media',
    'production reference obsolete media',
    'production postack unavailable media',
    'production postack scope media',
    'production postack unavailable',
    'production reference obsolete',
    'production accepted roster',
    'production multiple accepted',
    'production partial prefix',
    'production capacity',
    'production initial capacity',
    'production replacement',
    'equivalent config',
    'credential change',
    'relay change',
    'ordinary member',
    'ordinary invite',
    'fresh agent',
    'denied agent',
    'tainted unknown',
    'missing key',
    'consent change',
    'perwrite change',
    'revision change',
    'policy change',
    'accepted prefix',
    'accepted cancellation',
  ]) {
    testWidgets('fresh selected classification $scenario', (tester) async {
      final media = scenario.endsWith(' media');
      final mode = scenario.replaceAll(' media', '');
      final mixed = mode.startsWith('production mixed');
      final reference = mode.startsWith('production reference') || mixed;
      final signer = nostr.Keys.generate();
      final target = nostr.Keys.generate();
      final key = target.public;
      final production = mode.startsWith('production ');
      final multiple =
          mode == 'production multiple accepted' ||
          mode == 'production partial prefix' ||
          mixed;
      final secondSigner = nostr.Keys.generate();
      final second = secondSigner.public;
      final relay = nostr.Keys.generate();
      final acceptedKeys = <String>{};
      final gate = RelayRateLimitGate();
      late RelaySessionNotifier productionSession;
      NostrEvent rosterEvent() => signed(
        relay,
        39002,
        '',
        time: 100 + acceptedKeys.length,
        tags: [
          ['d', 'channel-1'],
          if (mode == 'production postack unavailable' &&
              acceptedKeys.isNotEmpty)
            ['d', 'channel-1'],
          ['p', signer.public],
          if (mixed) ['p', second, '', 'member'],
          for (final member in acceptedKeys) ['p', member, '', 'member'],
        ],
      );
      var profileChanged = false;
      final client = _selectedRosterClient(
        relay.public,
        rosterEvent,
        extraEvents: () => [
          if (profileChanged ||
              mode == 'production postack profile' && acceptedKeys.isNotEmpty)
            signed(target, 0, {}, time: 200),
          if (mode == 'production partial prefix' && acceptedKeys.isNotEmpty)
            signed(
              target,
              0,
              {},
              time: 200,
              tags: [
                ['auth', 'invalid-revoked-authority'],
              ],
            ),
        ],
      );
      final savedAgent = mode == 'tainted unknown';
      final events = <Map<String, dynamic>>[];
      final prefix = mode.startsWith('accepted');
      var reads = 0;
      var accepted = false;
      late TextEditingController controller;
      List<String>? sent;
      List<List<String>>? deliveredTags;
      var uploaded = false;
      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: signer.nsec,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async => null,
        pickGalleryImages: () async => [
          XFile.fromData(_pngBytes, name: 'tiny.png'),
        ],
        httpClient: http_testing.MockClient((_) async {
          uploaded = true;
          return http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/test.png',
              'sha256': '0' * 64,
              'size': 16,
              'type': 'image/png',
              'uploaded': 1,
            }),
            200,
          );
        }),
      );
      late ProviderContainer container;
      final roster = [
        ChannelMember(
          pubkey: key,
          displayName: 'Alice',
          role: 'member',
          joinedAt: DateTime(2025),
        ),
      ];
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: media ? service : _testUploadService(signer.nsec),
          currentPubkey: signer.public,
          rateLimitGate: production ? gate : null,
          relayHttpClient: production ? client : null,
          beforePublish: (event) {
            if (event.kind != 9000) return;
            if (mode == 'production invitation revoked') {
              productionSession.debugHandleSocketMessageForTest([
                'EVENT',
                'profile',
                signed(target, 0, {}, time: 200).toJson(),
              ]);
            }
            if (mode == 'production invitation draft' ||
                mode == 'production invitation capacity') {
              gate.activate(300);
            }
          },
          relayConfig: () => _SwitchableRelayConfigNotifier(
            RelayConfig(baseUrl: 'https://relay.example', nsec: signer.nsec),
          ),
          members: savedAgent
              ? []
              : [
                  ...roster,
                  if (multiple)
                    ChannelMember(
                      pubkey: second,
                      displayName: 'Bob',
                      role: 'member',
                      joinedAt: DateTime(2025),
                    ),
                ],
          relayAgents: savedAgent ? [_testAgent(key)] : [],
          channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
          selectedReader: production
              ? null
              : (keys, prior, viewer, channel, current, observed) async {
                  expect(keys, {key});
                  if (reads == 0) expect(prior, savedAgent ? {key} : isEmpty);
                  expect(current(), isTrue);
                  reads++;
                  if (mode == 'revision change') {
                    observed({
                      key: NostrEvent(
                        id: '$reads',
                        pubkey: key,
                        createdAt: reads,
                        kind: 0,
                        tags: [],
                        content: '{}',
                        sig: '',
                      ),
                    });
                  }
                  if (mode == 'missing key') return {};
                  final agent =
                      mode == 'fresh agent' ||
                      mode == 'denied agent' ||
                      mode == 'policy change' ||
                      (mode == 'consent change' && reads >= 2) ||
                      (mode == 'perwrite change' && reads >= 3);
                  return {
                    key: SelectedMentionAuthorization(
                      savedAgent || prefix && accepted
                          ? SelectedMentionKind.unresolvedAgent
                          : agent
                          ? SelectedMentionKind.agent
                          : SelectedMentionKind.ordinary,
                      mode == 'ordinary member' || accepted,
                      agent
                          ? AgentDirectoryEntry(
                              pubkey: key,
                              ownerPubkey: viewer,
                              respondTo:
                                  (mode == 'denied agent' ||
                                      mode == 'policy change' && reads >= 2)
                                  ? 'nobody'
                                  : 'anyone',
                              channelIds: accepted ? [channel] : [],
                            )
                          : null,
                    ),
                  };
                },
          onSend: (_, keys, {mediaTags = const []}) async {
            if (reference) {
              productionSession.debugHandleSocketMessageForTest([
                'EVENT',
                'profile',
                signed(
                  mode == 'production mixed revoked' ? secondSigner : target,
                  0,
                  {},
                  time: 200,
                  tags: [
                    if (mode == 'production mixed revoked')
                      ['auth', 'invalid-revoked-authority'],
                  ],
                ).toJson(),
              ]);
            }
            sent = keys;
            deliveredTags = mediaTags;
            if (production) {
              await SendMessage(
                signedEventRelay: SignedEventRelay(
                  session: productionSession,
                  nsec: signer.nsec,
                ),
                fetchMembers: (_) async => [],
                readUserCache: () => {},
                isDeliveryValid: () => true,
                completeLocalMessage: (_, _) {},
                removeLocalMessage: (_, _) {},
                addLocalMessage: (_, _) {
                  if (mode == 'production final revoked') {
                    productionSession.debugHandleSocketMessageForTest([
                      'EVENT',
                      'profile',
                      signed(target, 0, {}, time: 200).toJson(),
                    ]);
                  }
                  if (mode == 'production final waiting revocation' ||
                      mode == 'production final capacity' ||
                      mode == 'production final draft') {
                    gate.activate(300);
                  }
                },
              )(
                channelId: 'channel-1',
                content: '',
                mentionPubkeys: keys,
                mediaTags: mediaTags,
              );
            }
          },
        ),
      );
      container = ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      );
      final session = container.read(relaySessionProvider.notifier);
      productionSession = session;
      session.debugAttachSocketForTest(
        _RecordingRelaySocket(
          events,
          session.debugHandleSocketMessageForTest,
          onEventAcknowledged: (event) {
            if (event['kind'] != 9000) return;
            accepted = true;
            if (production) {
              acceptedKeys.add(
                (event['tags'] as List).firstWhere((t) => t[0] == 'p')[1]
                    as String,
              );
              session.debugHandleSocketMessageForTest([
                'EVENT',
                'roster',
                rosterEvent().toJson(),
              ]);
            }
            if ([
              'production postack scope',
              'equivalent config',
              'credential change',
              'relay change',
            ].contains(mode)) {
              container
                  .read(relayConfigProvider.notifier)
                  .update(
                    baseUrl:
                        mode == 'relay change' ||
                            mode == 'production postack scope'
                        ? 'https://other.example'
                        : 'https://relay.example',
                    nsec: mode == 'credential change'
                        ? nostr.Keys.generate().nsec
                        : signer.nsec,
                  );
            }
            if (mode == 'accepted cancellation') {
              controller.text = 'new draft';
            }
          },
        ),
      );
      if (media) {
        await _openSystemPhotoPicker(tester);
        await tester.pumpAndSettle();
      }
      await _expandComposer(tester);
      await tester.enterText(
        find.byType(TextField),
        savedAgent ? '@hel' : '@ali',
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text(savedAgent ? 'Helper Bot' : 'Alice'));
      await tester.pumpAndSettle();
      controller = tester.widget<TextField>(find.byType(TextField)).controller!;
      if (multiple) {
        await tester.enterText(find.byType(TextField), '${controller.text}@bo');
        await tester.pumpAndSettle();
        await tester.tap(find.text('Bob'));
        await tester.pumpAndSettle();
      }
      final draft = controller.text;
      if (mode == 'production initial capacity') gate.activate(300);
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      if (find.text('Invite').evaluate().isNotEmpty) {
        expect(events.where((event) => event['kind'] == 9000), isEmpty);
        if (mode == 'production capacity') gate.activate(300);
        if (mode == 'production replacement') {
          session.debugSupersedeConnection();
        }
        if (mode == 'production reference scope') {
          container
              .read(relayConfigProvider.notifier)
              .update(baseUrl: 'https://other.example', nsec: signer.nsec);
        }
        if (mode == 'production consent profile') profileChanged = true;
        await tester.tap(
          find.text(reference ? 'Send without inviting' : 'Invite'),
        );
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 300));
      }
      if (mode == 'production invitation draft') {
        expect(events.where((e) => e['kind'] == 9000), isEmpty);
        controller.text = 'new draft';
      }
      if (mode.startsWith('production final')) {
        expect(sent, [key]);
        expect(events.where((e) => e['kind'] == 9), isEmpty);
        if (mode == 'production final draft') controller.text = 'new draft';
      }
      if (mode == 'production final waiting revocation') {
        productionSession.debugHandleSocketMessageForTest([
          'EVENT',
          'profile',
          signed(target, 0, {}, time: 200).toJson(),
        ]);
      }
      gate.reset();
      if (media) {
        await tester.runAsync(() async {
          await Future<void>.delayed(const Duration(milliseconds: 150));
        });
      }
      await tester.pumpAndSettle();
      if (production) {
        final succeeds =
            mode == 'production reference obsolete' ||
            mode == 'production mixed obsolete' ||
            mode == 'production accepted roster' ||
            mode == 'production multiple accepted';
        expect(events.where((e) => e['kind'] == 9).length, succeeds ? 1 : 0);
        expect(
          acceptedKeys.length,
          succeeds
              ? (reference
                    ? 0
                    : multiple
                    ? 2
                    : 1)
              : (mode == 'production partial prefix' ||
                    mode == 'production postack unavailable' ||
                    mode == 'production postack scope' ||
                    mode == 'production postack profile' ||
                    mode.startsWith('production final'))
              ? 1
              : 0,
        );
        if (media) expect(uploaded, isTrue);
        if (reference && !mode.endsWith(' scope')) {
          expect(sent, mixed ? [second] : isEmpty);
          expect(deliveredTags, contains(equals(['mention', key])));
        } else if (deliveredTags != null) {
          expect(
            deliveredTags!.where((tag) => tag.first == 'mention'),
            isEmpty,
          );
        }
        if (media && deliveredTags != null) {
          expect(deliveredTags!.any((tag) => tag.first == 'imeta'), isTrue);
        }
        if (mode == 'production mixed revoked') {
          expect(
            find.textContaining('Mention evidence changed; retry the draft'),
            findsOneWidget,
          );
        }
        if (mode == 'production postack unavailable') {
          expect(
            find.textContaining('1 invitation(s) completed and remain'),
            findsOneWidget,
          );
          expect(
            find.text('Message not sent: the community changed'),
            findsNothing,
          );
        }
        if (mode.endsWith(' scope')) {
          expect(
            find.text('Message not sent: the community changed'),
            findsOneWidget,
          );
        }
        if (reference && succeeds) {
          expect(controller.text, isEmpty);
          return;
        }
        if (succeeds) {
          expect(sent, multiple ? [key, second] : [key]);
        } else {
          expect(
            controller.text,
            mode.endsWith(' scope')
                ? ''
                : mode.endsWith(' draft')
                ? 'new draft'
                : draft,
          );
          if (!mode.endsWith(' draft') &&
              (!media || mode != 'production mixed revoked')) {
            expect(find.byType(SnackBar), findsOneWidget);
          }
        }
        return;
      }
      if (deliveredTags != null) {
        expect(deliveredTags!.where((tag) => tag.first == 'mention'), isEmpty);
      }
      if (mode == 'revision change' || mode == 'policy change') {
        expect(reads, 2);
      }
      if (mode == 'consent change' || mode == 'perwrite change') {
        expect(reads, 3);
      }
      final succeeds = [
        'equivalent config',
        'ordinary member',
        'ordinary invite',
        'fresh agent',
      ].contains(mode);
      expect(sent, succeeds ? [key] : isNull);
      final writes = events.where((event) => event['kind'] == 9000).toList();
      expect(
        writes,
        hasLength(
          [
                    'ordinary invite',
                    'fresh agent',
                    'equivalent config',
                    'credential change',
                    'relay change',
                  ].contains(mode) ||
                  prefix
              ? 1
              : 0,
        ),
      );
      if (writes.isNotEmpty) {
        expect(
          (writes.single['tags'] as List).where((tag) => tag[0] == 'p').single,
          ['p', key],
        );
        expect(
          writes.single['tags'],
          contains(equals(['role', mode == 'fresh agent' ? 'bot' : 'member'])),
        );
      }
      if (!succeeds) {
        expect(
          controller.text,
          ['credential change', 'relay change'].contains(mode)
              ? '' // The new identity owns a separate empty composer.
              : mode == 'accepted cancellation'
              ? 'new draft'
              : savedAgent
              ? '@Helper Bot '
              : '@Alice ',
        );
      }
      if (prefix) {
        expect(
          find.textContaining('1 invitation(s) completed and remain'),
          findsOneWidget,
        );
      }
    });
  }
}
