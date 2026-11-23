part of '../reaction_row_test.dart';

void combinedReactionTests() {
  for (final selected in [false, true]) {
    for (final count in [1, 2]) {
      testWidgets(
        'combined tray has one actionable semantics label, selected=$selected count=$count',
        (tester) async {
          final semantics = tester.ensureSemantics();

          await _pumpRow(
            tester,
            combined: true,
            reactions: [
              _reaction(count: count, reactedByCurrentUser: selected),
            ],
            onToggle: (_) {},
          );
          final label =
              'View $count ${count == 1 ? 'reaction' : 'reactions'}${selected ? ', including yours' : ''}';
          final tray = find.bySemanticsLabel(label);
          expect(tray, findsOneWidget);
          final node = tester.getSemantics(tray);
          expect(node.label, label);
          expect(
            node.getSemanticsData().hasAction(SemanticsAction.tap),
            isTrue,
          );
          expect(node.childrenCount, 0);
          semantics.dispose();
        },
      );
    }
  }

  testWidgets('Android combined tray removes own reaction and stays live', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final actions = _RecordingReactionActions();
    Future<void> draw(bool selected) async {
      final message = TimelineMessage(
        id: _messageId,
        pubkey: _alice,
        createdAt: 1,
        content: '',
        reactions: [
          TimelineReaction(
            emoji: _fire,
            count: selected ? 2 : 1,
            reactedByCurrentUser: selected,
            currentUserReactionId: selected ? 'own-reaction' : null,
            userPubkeys: selected ? [_alice, 'other'] : ['other'],
          ),
        ],
      );
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            channelActionsProvider.overrideWithValue(actions),
            userCacheProvider.overrideWith(() => _FakeUserCacheNotifier({})),
          ],
          child: Consumer(
            builder: (context, ref, _) => ReactionRow(
              combined: true,
              messageId: message.id,
              channelId: 'channel',
              reactions: message.reactions,
              onToggle: (emoji) => toggleReaction(
                context,
                ProviderScope.containerOf(context, listen: false),
                message,
                emoji,
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
    }

    await draw(true);
    await tester.tap(find.byKey(const ValueKey('combined-reaction-tray')));
    await tester.pumpAndSettle();
    final chip = find.byKey(const ValueKey('reaction-toggle-$_fire'));
    expect(tester.widget<FilterChip>(chip).selected, isTrue);
    await tester.tap(chip);
    await tester.pump();
    expect(actions.removed, ['own-reaction']);
    expect(actions.added, isEmpty);
    await draw(false);
    expect(tester.widget<FilterChip>(chip).selected, isFalse);
    expect(
      find.byKey(const ValueKey('reaction-details-sheet')),
      findsOneWidget,
    );
    expect(find.text('2'), findsNothing);
    expect(find.text('1'), findsNWidgets(2));
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('native tray Add still opens the picker after the row is '
      'rebuilt away', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    resetIosEmojiPickerPresentationForTest();
    final completion = Completer<Map<String, Object?>>();
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeMessagePresentation.channel,
      (call) async =>
          call.method == 'reactions' ? completion.future : <String, Object?>{},
    );
    final pickerCalls = <String>[];
    const picker = MethodChannel('buzz/native_emoji_picker');
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(picker, (
      call,
    ) async {
      pickerCalls.add(call.method);
      return true;
    });
    addTearDown(() {
      tester.binding.defaultBinaryMessenger
        ..setMockMethodCallHandler(NativeMessagePresentation.channel, null)
        ..setMockMethodCallHandler(picker, null);
    });
    final message = TimelineMessage(
      id: _messageId,
      pubkey: _alice,
      createdAt: 1,
      content: '',
      reactions: [
        _reaction(count: 1, userPubkeys: const ['other']),
      ],
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final rowMounted = ValueNotifier(true);
    addTearDown(rowMounted.dispose);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
          channelActionsProvider.overrideWithValue(_RecordingReactionActions()),
          userCacheProvider.overrideWith(() => _FakeUserCacheNotifier({})),
        ],
        child: ValueListenableBuilder<bool>(
          valueListenable: rowMounted,
          builder: (_, mounted, _) => mounted
              ? Builder(
                  builder: (context) => ReactionRow(
                    combined: true,
                    messageId: message.id,
                    channelId: 'channel',
                    reactions: message.reactions,
                    onToggle: (_) {},
                    showAddButton: true,
                    onAddReaction: addReactionPickerFor(
                      context: context,
                      message: message,
                    ),
                  ),
                )
              : const SizedBox.shrink(),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('combined-reaction-tray')));
    await tester.pump();

    // A new message shifts the timeline while the native tray is open.
    rowMounted.value = false;
    await tester.pump();
    completion.complete({'action': 'addReaction'});
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
    expect(pickerCalls, contains('present'));
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('tray picks made after the active community switched are '
      'dropped', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    resetIosEmojiPickerPresentationForTest();
    final completion = Completer<Map<String, Object?>>();
    Map<Object?, Object?>? payload;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeMessagePresentation.channel,
      (call) async {
        if (call.method != 'reactions') return <String, Object?>{};
        payload = call.arguments as Map<Object?, Object?>;
        return completion.future;
      },
    );
    final pickerCalls = <String>[];
    const picker = MethodChannel('buzz/native_emoji_picker');
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(picker, (
      call,
    ) async {
      pickerCalls.add(call.method);
      return true;
    });
    addTearDown(() {
      tester.binding.defaultBinaryMessenger
        ..setMockMethodCallHandler(NativeMessagePresentation.channel, null)
        ..setMockMethodCallHandler(picker, null);
    });
    final message = TimelineMessage(
      id: _messageId,
      pubkey: _alice,
      createdAt: 1,
      content: '',
      reactions: [
        _reaction(count: 1, userPubkeys: const ['other']),
      ],
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final actions = _RecordingReactionActions();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
          channelActionsProvider.overrideWithValue(actions),
          userCacheProvider.overrideWith(() => _FakeUserCacheNotifier({})),
        ],
        child: Builder(
          builder: (context) => ReactionRow(
            combined: true,
            messageId: message.id,
            channelId: 'channel',
            reactions: message.reactions,
            onToggle: (emoji) => toggleReaction(
              context,
              ProviderScope.containerOf(context, listen: false),
              message,
              emoji,
            ),
            showAddButton: true,
            onAddReaction: addReactionPickerFor(
              context: context,
              message: message,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ReactionRow)),
      listen: false,
    );
    final recentsBefore = container.read(recentEmojiProvider);
    await tester.tap(find.byKey(const ValueKey('combined-reaction-tray')));
    await tester.pump();

    // A notification tap moves the app to another community while the
    // native tray is still up.
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://other.example');
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      NativeMessagePresentation.channel.name,
      const StandardMethodCodec().encodeMethodCall(
        MethodCall('toggleReaction', {
          'requestId': payload!['requestId'],
          'emoji': _fire,
        }),
      ),
      (_) {},
    );
    completion.complete({'action': 'addReaction'});
    await tester.pumpAndSettle();

    expect(actions.added, isEmpty);
    expect(pickerCalls, isNot(contains('present')));
    expect(container.read(recentEmojiProvider), recentsBefore);
    expect(container.read(pendingReactionBurstProvider), isNull);
    debugDefaultTargetPlatformOverride = null;
  });

  for (final loadCustom in [false, true]) {
    for (final scale in [1.0, 3.0]) {
      testWidgets(
        'mixed reaction glyphs stay centered, image=$loadCustom scale=$scale',
        (tester) async {
          final imageBytes = (await rootBundle.load(
            'assets/images/buzz-icon.png',
          )).buffer.asUint8List();
          final client = MockClient(
            (_) async => loadCustom
                ? http.Response.bytes(
                    imageBytes,
                    200,
                    headers: {'content-type': 'image/png'},
                  )
                : http.Response('missing', 404),
          );
          addTearDown(client.close);
          await tester.pumpWidget(
            WidgetHelpers.testable(
              overrides: [mediaHttpClientProvider.overrideWithValue(client)],
              child: MediaQuery(
                data: MediaQueryData(textScaler: TextScaler.linear(scale)),
                child: ReactionRow(
                  combined: true,
                  messageId: 'alignment',
                  channelId: 'channel',
                  onToggle: (_) {},
                  reactions: [
                    for (final emoji in [
                      '🙂',
                      '👀',
                      '❤️',
                      '👩‍👩‍👧‍👦',
                      '🇺🇸',
                      '✅',
                      '🎥',
                      '🎯',
                      '🙌',
                    ])
                      _reaction(emoji: emoji),
                    TimelineReaction(
                      emoji: ':custom:',
                      emojiUrl:
                          'https://example.com/align-$loadCustom-$scale.png',
                      count: 1,
                      reactedByCurrentUser: false,
                      userPubkeys: const [_alice],
                    ),
                  ],
                ),
              ),
            ),
          );
          await tester.pumpAndSettle();
          if (loadCustom) {
            final source = tester.widget<Image>(
              find.descendant(
                of: find.byType(CustomEmojiImage),
                matching: find.byType(Image),
              ),
            );
            await tester.runAsync(
              () => precacheImage(
                source.image,
                tester.element(find.byType(CustomEmojiImage)),
              ),
            );
            await tester.pumpAndSettle();
            final image = tester.widget<RawImage>(
              find.descendant(
                of: find.byType(CustomEmojiImage),
                matching: find.byType(RawImage),
              ),
            );
            expect(image.image, isNotNull);
          } else {
            // A truncated shortcode is illegible in a pill: show sparkles.
            expect(find.byIcon(BuzzIcons.sparkles), findsOneWidget);
            expect(find.text(':custom:'), findsNothing);
          }
          final tray = find.byKey(const ValueKey('combined-reaction-tray'));
          final center = tester.getCenter(tray).dy;
          for (final glyph in find.byType(NativeEmojiGlyph).evaluate()) {
            final finder = find.byWidget(glyph.widget);
            expect(tester.getSize(finder), const Size.square(20));
            expect(tester.getCenter(finder).dy, closeTo(center, 0.01));
            final paintedText = find.descendant(
              of: finder,
              matching: find.byType(Text),
            );
            // This checks the device-calibrated optical correction, not the
            // pixel bounds of the font (host rendering differs from iOS).
            final opticalLift = defaultTargetPlatform == TargetPlatform.iOS
                ? 2.0
                : 0.0;
            expect(
              tester.getCenter(paintedText).dy,
              closeTo(center - opticalLift, 0.01),
            );
          }
          final customBox = find
              .ancestor(
                of: find.byType(CustomEmojiImage),
                matching: find.byType(SizedBox),
              )
              .first;
          expect(tester.getSize(customBox), const Size.square(20));
          expect(tester.getCenter(customBox).dy, closeTo(center, 0.01));
          expect(tester.getCenter(find.text('10')).dy, closeTo(center, 0.01));
          expect(tester.takeException(), isNull);
        },
        variant: TargetPlatformVariant({
          TargetPlatform.iOS,
          TargetPlatform.android,
        }),
      );
    }
  }

  testWidgets(
    'combined capsule shows total count and opens details without toggling',
    (tester) async {
      final toggled = <String>[];
      var added = false;
      await _pumpRow(
        tester,
        combined: true,
        reactions: [
          _reaction(count: 3),
          _reaction(emoji: _eyes),
        ],
        onToggle: toggled.add,
        showAddButton: true,
        onAddReaction: () => added = true,
      );
      expect(find.text('1'), findsNothing);
      expect(find.text('4'), findsOneWidget);
      expect(find.byKey(const ValueKey('add-reaction-pill')), findsNothing);
      expect(
        tester
            .getSize(find.byKey(const ValueKey('combined-reaction-tray')))
            .width,
        lessThan(100),
      );
      await tester.tap(find.byKey(const ValueKey('combined-reaction-tray')));
      await tester.pumpAndSettle();
      final sheet = find.byKey(const ValueKey('reaction-details-sheet'));
      expect(sheet, findsOneWidget);
      expect(toggled, isEmpty);
      await tester.tap(find.byKey(const ValueKey('reaction-sheet-add')));
      await tester.pumpAndSettle();
      expect(added, isTrue);
    },
  );

  testWidgets('native add action waits for reaction sheet dismissal', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final completion = Completer<Map<String, Object?>>();
    Map<Object?, Object?>? payload;
    var added = 0;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeMessagePresentation.channel,
      (call) async {
        if (call.method == 'reactions') {
          payload = call.arguments as Map<Object?, Object?>;
          return completion.future;
        }
        return <String, Object?>{};
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        null,
      ),
    );
    await _pumpRow(
      tester,
      combined: true,
      reactions: [_reaction()],
      showAddButton: true,
      onAddReaction: () => added++,
    );
    await tester.tap(find.byKey(const ValueKey('combined-reaction-tray')));
    await tester.pump();
    expect(payload!['canAddReaction'], isTrue);
    expect(payload!['addReactionIconCodePoint'], BuzzIcons.smilePlus.codePoint);
    await NativeMessagePresentation.present('updateProfiles', {
      'requestId': payload!['requestId'],
      'profiles': <String, Object?>{},
    });
    expect(added, 0);
    completion.complete({'action': 'addReaction'});
    await tester.pumpAndSettle();
    expect(added, 1);
    debugDefaultTargetPlatformOverride = null;
  });

  Future<void> pumpFailingTray(WidgetTester tester) async {
    final message = TimelineMessage(
      id: _messageId,
      pubkey: _alice,
      createdAt: 1,
      content: '',
      reactions: [
        TimelineReaction(
          emoji: _fire,
          count: 1,
          reactedByCurrentUser: true,
          currentUserReactionId: 'own-reaction',
          userPubkeys: const [_alice],
        ),
      ],
    );
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          channelActionsProvider.overrideWithValue(
            _FailingReactionActions(RelayNotConnectedError()),
          ),
          userCacheProvider.overrideWith(() => _FakeUserCacheNotifier({})),
        ],
        child: Consumer(
          builder: (context, ref, _) => ReactionRow(
            combined: true,
            messageId: message.id,
            channelId: 'channel',
            reactions: message.reactions,
            onToggle: (emoji) => toggleReaction(
              context,
              ProviderScope.containerOf(context, listen: false),
              message,
              emoji,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('a failed toggle in the details sheet shows above the sheet', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await pumpFailingTray(tester);
    await tester.tap(find.byKey(const ValueKey('combined-reaction-tray')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('reaction-toggle-$_fire')));
    await tester.pump();
    expect(
      find.byKey(const ValueKey('reaction-details-sheet')),
      findsOneWidget,
    );
    // A page snackbar would sit in the Scaffold, under the sheet's route.
    final notice = find.text('Couldn’t remove reaction. No connection.');
    expect(notice, findsOneWidget);
    expect(
      find.ancestor(of: notice, matching: find.byType(Scaffold)),
      findsNothing,
    );
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('a failed toggle in the native sheet shows on that sheet', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final completion = Completer<Map<String, Object?>>();
    Object? requestId;
    final notices = <Map<Object?, Object?>>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeMessagePresentation.channel,
      (call) async {
        final data = call.arguments as Map<Object?, Object?>;
        if (call.method == 'reactions') {
          requestId = data['requestId'];
          return completion.future;
        }
        if (call.method == 'notice') notices.add(data);
        return <String, Object?>{};
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        null,
      ),
    );
    await pumpFailingTray(tester);
    await tester.tap(find.byKey(const ValueKey('combined-reaction-tray')));
    await tester.pump();
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      NativeMessagePresentation.channel.name,
      const StandardMethodCodec().encodeMethodCall(
        MethodCall('toggleReaction', {'requestId': requestId, 'emoji': _fire}),
      ),
      (_) {},
    );
    await tester.pump();
    expect(notices, [
      {
        'requestId': requestId,
        'message': 'Couldn’t remove reaction. No connection.',
      },
    ]);
    expect(find.text('Couldn’t remove reaction. No connection.'), findsNothing);
    completion.complete(<String, Object?>{});
    await tester.pumpAndSettle();
    debugDefaultTargetPlatformOverride = null;
  });
}
