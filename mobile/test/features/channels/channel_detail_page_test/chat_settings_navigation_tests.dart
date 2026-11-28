part of '../channel_detail_page_test.dart';

void chatSettingsNavigationTests() {
  for (final supportsZoom in [true, false]) {
    for (final groupDm in [false, true]) {
      testWidgets(
        'native ${groupDm ? 'group' : 'direct'} DM title opens settings'
        ' (phone zoom: $supportsZoom)',
        (tester) async {
          final messenger = tester.binding.defaultBinaryMessenger;
          const actions = MethodChannel('buzz/channel_actions_sheet');
          const media = MethodChannel('buzz/media_upload');
          final platformViews = <MethodChannel>[];
          MethodChannel? chatNav;
          final presented = Completer<String?>();
          Map<Object?, Object?>? presentation;
          final restoredSessions = <Object?>[];
          Map<Object?, Object?>? navConfiguration;
          messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
            call,
          ) async {
            if (call.method != 'create') return null;
            final arguments = call.arguments as Map;
            final viewType = arguments['viewType'];
            final bridge = MethodChannel('$viewType/${arguments['id']}');
            platformViews.add(bridge);
            if (viewType == IosNavigationBar.viewType) chatNav ??= bridge;
            messenger.setMockMethodCallHandler(bridge, (call) async {
              if (bridge == chatNav && call.method == 'configure') {
                navConfiguration = call.arguments as Map<Object?, Object?>;
              }
              return null;
            });
            return null;
          });
          messenger.setMockMethodCallHandler(media, (call) async {
            if (call.method == 'clipboardHasImage') return false;
            throw MissingPluginException();
          });
          messenger.setMockMethodCallHandler(actions, (call) async {
            if (call.method == 'supportsChatSettingsZoom') return supportsZoom;
            if (!supportsZoom) {
              fail('Unexpected native settings call: ${call.method}');
            }
            switch (call.method) {
              case 'present':
                presentation = call.arguments as Map<Object?, Object?>;
                return presented.future;
              case 'sourceRestored':
                restoredSessions.add((call.arguments as Map)['session']);
                return null;
              default:
                expect(call.method, anyOf('update', 'dismiss'));
                return null;
            }
          });
          addTearDown(() {
            for (final platformView in platformViews) {
              messenger.setMockMethodCallHandler(platformView, null);
            }
            messenger.setMockMethodCallHandler(
              SystemChannels.platform_views,
              null,
            );
            messenger.setMockMethodCallHandler(media, null);
            messenger.setMockMethodCallHandler(actions, null);
          });

          tester.view.devicePixelRatio = 1;
          tester.view.physicalSize = const Size(402, 874);
          addTearDown(tester.view.reset);
          final participants = ['self', 'alice', if (groupDm) 'bob'];
          final dm = Channel(
            id: _channelId,
            name: 'DM',
            channelType: 'dm',
            visibility: 'private',
            description: '',
            createdBy: 'self',
            createdAt: DateTime(2025),
            memberCount: participants.length,
            participants: participants,
            participantPubkeys: participants,
            isMember: true,
          );
          final relay = _SettingsDmRelay();
          final navigator = _SettingsRouteObserver();
          await tester.pumpWidget(
            _buildTestable(
              messages: const [],
              channel: dm,
              huddleCurrentPubkey: 'self',
              navigatorObservers: [navigator],
              relayConfigNotifier: _LinkedProfileConfig(),
              relaySessionNotifier: relay,
              members: [
                for (final pubkey in participants)
                  ChannelMember(
                    pubkey: pubkey,
                    role: 'member',
                    joinedAt: DateTime(2025),
                  ),
              ],
              users: const {
                'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
                'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
              },
              extraOverrides: [
                channelMessagesProvider(_otherChannelId).overrideWith(
                  () => _FakeMessagesNotifier(
                    const [],
                    channelId: _otherChannelId,
                  ),
                ),
              ],
            ),
          );
          await tester.pumpAndSettle();
          final nativeBar = tester.widget<UiKitView>(
            find.byWidgetPredicate(
              (widget) =>
                  widget is UiKitView &&
                  widget.viewType == IosNavigationBar.viewType,
            ),
          );
          final header = nativeBar.creationParams! as Map;
          expect(header['titleEnabled'], isTrue);
          expect(header['subtitle'], '');
          expect(header['titlePresenceColor'], isNull);
          expect(
            (header['actions'] as List).cast<Map>().map((a) => a['symbol']),
            isNot(contains(anyOf('person.2', 'ellipsis'))),
          );
          final navBridge = chatNav!;
          final portraitBounds = tester.getRect(find.byType(DmHeaderCluster));
          Future<ByteData?> pressTitle() => messenger.handlePlatformMessage(
            navBridge.name,
            navBridge.codec.encodeMethodCall(
              const MethodCall('titlePressed', [160.0, 65.0, 82.0, 28.0]),
            ),
            null,
          );
          if (supportsZoom) {
            // The engine rasterizes the portrait asynchronously. Wait for the
            // route push with real async enabled, then pump its UI.
            final pushed = await tester.runAsync(() async {
              final nextRoute = navigator.nextPush();
              await pressTitle();
              return nextRoute.timeout(const Duration(seconds: 5));
            });
            expect(pushed, isNotNull);
          } else {
            await pressTitle();
          }
          await tester.pumpAndSettle();

          final title = groupDm ? 'Alice, Bob' : 'Alice';
          final details = find.byType(ChannelDetailsPage);
          expect(details, findsOneWidget);
          expect(tester.widget<ChannelDetailsPage>(details).channel.id, dm.id);
          if (!supportsZoom) {
            expect(presentation, isNull);
            expect(
              find.descendant(
                of: find.byKey(const ValueKey('channel-details-name')),
                matching: find.text(title),
              ),
              findsOneWidget,
            );
            expect(
              find.byKey(const ValueKey('dm-details-avatar')),
              groupDm ? findsNothing : findsOneWidget,
            );
            expect(
              find.byKey(const ValueKey('channel-details-members-card')),
              groupDm ? findsOneWidget : findsNothing,
            );
          } else {
            expect(presentation, isNotNull);
            final payload = presentation!;
            expect(payload['title'], title);
            expect(payload['holdsZoomSource'], isTrue);
            expect(payload['zoomSource'], [
              portraitBounds.left,
              portraitBounds.top,
              portraitBounds.width,
              portraitBounds.height,
            ]);
            expect((payload['zoomArtwork'] as Uint8List).take(8), [
              137,
              80,
              78,
              71,
              13,
              10,
              26,
              10,
            ]);
            final session = payload['session'];
            final source = MethodChannel('${actions.name}/source/$session');
            final covering = messenger.handlePlatformMessage(
              source.name,
              source.codec.encodeMethodCall(const MethodCall('covered')),
              null,
            );
            await tester.pumpAndSettle();
            await covering;
            expect(navConfiguration, containsPair('hideSettingsSource', true));
            final composer = tester.element(find.byType(ComposeBar));
            expect(TickerMode.valuesOf(composer).enabled, isFalse);

            if (groupDm) {
              final memberRows = (payload['sections'] as List)
                  .cast<List>()
                  .expand((rows) => rows.cast<Map<Object?, Object?>>());
              expect(
                memberRows,
                contains(
                  allOf(
                    containsPair('id', 'member:alice'),
                    containsPair('enabled', true),
                  ),
                ),
              );
              presented.complete('member:alice');
            } else {
              presented.complete();
            }
            await tester.pumpAndSettle();
            expect(restoredSessions, [session]);
            expect(navConfiguration, containsPair('hideSettingsSource', false));

            if (groupDm) {
              final profile = find.byType(UserProfileSheet);
              expect(tester.widget<UserProfileSheet>(profile).pubkey, 'alice');
              final messageButton = find.descendant(
                of: profile,
                matching: find.text('Message'),
              );
              await tester.ensureVisible(messageButton);
              await tester.tap(messageButton);
              await tester.pumpAndSettle();
              expect(relay.dmRequests.single.tags, [
                ['p', 'alice'],
              ]);
              expect(profile, findsNothing);
              final conversation = find.byType(ChannelDetailPage);
              expect(
                tester.widget<ChannelDetailPage>(conversation).channel.id,
                _otherChannelId,
              );
              expect(find.byType(ComposeBar), findsOneWidget);
              Navigator.of(tester.element(conversation)).pop();
              await tester.pumpAndSettle();
              expect(details, findsOneWidget);
            } else {
              expect(details, findsNothing);
              expect(find.byType(ChannelDetailPage), findsOneWidget);
              expect(TickerMode.valuesOf(composer).enabled, isTrue);
            }
          }
          expect(tester.takeException(), isNull);
          await tester.pumpWidget(const SizedBox.shrink());
          await tester.pumpAndSettle();
        },
        variant: const TargetPlatformVariant({TargetPlatform.iOS}),
      );
    }
  }
}

class _SettingsRouteObserver extends NavigatorObserver {
  Completer<Route<dynamic>>? _nextPush;

  Future<Route<dynamic>> nextPush() {
    _nextPush = Completer<Route<dynamic>>();
    return _nextPush!.future;
  }

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    _nextPush?.complete(route);
    _nextPush = null;
  }
}

class _SettingsDmRelay extends _IdentityUpdateRelaySession {
  final dmRequests = <NostrEvent>[];

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
    Future<bool> Function()? mayPublish,
  }) async {
    if (event.kind != 41010) return event;
    dmRequests.add(event);
    return NostrEvent(
      id: event.id,
      pubkey: event.pubkey,
      createdAt: event.createdAt,
      kind: event.kind,
      tags: event.tags,
      content: 'response:{"channel_id":"$_otherChannelId","created":true}',
      sig: event.sig,
    );
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.tags['#d']?.contains(_otherChannelId) != true) {
      return super.fetchHistory(filter, timeout: timeout);
    }
    final tagsByKind = {
      39000: [
        ['name', 'DM'],
        ['t', 'dm'],
        ['private'],
        ['p', 'self'],
        ['p', 'alice'],
      ],
      39002: [
        ['p', 'self', '', 'member'],
        ['p', 'alice', '', 'member'],
      ],
    };
    return [
      for (final kind in filter.kinds)
        if (tagsByKind[kind] case final tags?)
          NostrEvent(
            id: '$kind-$_otherChannelId',
            pubkey: 'relay',
            createdAt: 1,
            kind: kind,
            tags: [
              ['d', _otherChannelId],
              ...tags,
            ],
            content: '',
            sig: '',
          ),
    ];
  }
}
