part of 'message_actions_test.dart';

const _nativeEmojiPicker = MethodChannel('buzz/native_emoji_picker');

class _ReadyCustomEmojiPalette extends CustomEmojiPaletteNotifier {
  @override
  Future<List<CustomEmoji>> build() async => const [];
}

Future<void> _sendNativeEmojiPicker(
  WidgetTester tester,
  String method, [
  Object? arguments,
]) async {
  await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    _nativeEmojiPicker.name,
    _nativeEmojiPicker.codec.encodeMethodCall(MethodCall(method, arguments)),
    (_) {},
  );
  await tester.pumpAndSettle();
}

void _messageActionsRowRebuildTests() {
  group('a timeline rebuild while reacting', () {
    setUp(resetIosEmojiPickerPresentationForTest);
    tearDown(() {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_nativeEmojiPicker, null);
      resetIosEmojiPickerPresentationForTest();
    });

    for (final viaPicker in [false, true]) {
      testWidgets('a ${viaPicker ? '"+" pick' : 'quick tile'} made after the '
          'active community switched is dropped', (tester) async {
        final previousPlatform = debugDefaultTargetPlatformOverride;
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        try {
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
              .setMockMethodCallHandler(_nativeEmojiPicker, (_) async => true);
          final actions = _RecordingReactionActions();
          final harness = await _pumpMessageActionsPopover(
            tester,
            message: _message(),
            prefs: await _mockPrefs(),
            createChannelActions: (_) => actions,
            overrides: [
              customEmojiPaletteProvider.overrideWith(
                _ReadyCustomEmojiPalette.new,
              ),
            ],
          );
          final recentsBefore = harness.container.read(recentEmojiProvider);

          // A notification tap moves the app to another community while the
          // actions sheet is still up.
          harness.container
              .read(relayConfigProvider.notifier)
              .update(baseUrl: 'https://other.example');
          await tester.pump();
          if (viaPicker) {
            await tester.tap(find.byKey(const ValueKey('quick-reaction-more')));
            await tester.pumpAndSettle();
            await _sendNativeEmojiPicker(tester, 'selected', '🎉');
            await _sendNativeEmojiPicker(tester, 'dismissed');
          } else {
            await tester.tap(find.byKey(const ValueKey('quick-reaction-👍')));
            await tester.pumpAndSettle();
          }

          expect(actions.added, isEmpty);
          expect(harness.container.read(recentEmojiProvider), recentsBefore);
          expect(harness.container.read(pendingReactionBurstProvider), isNull);
          expect(tester.takeException(), isNull);
        } finally {
          debugDefaultTargetPlatformOverride = previousPlatform;
        }
      });
    }

    for (final unmountWhilePickerOpen in [false, true]) {
      testWidgets('the "+" pick still reacts when the row unmounts '
          '${unmountWhilePickerOpen ? 'while the picker is open' : 'first'}', (
        tester,
      ) async {
        final previousPlatform = debugDefaultTargetPlatformOverride;
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        try {
          var presents = 0;
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
              .setMockMethodCallHandler(_nativeEmojiPicker, (call) async {
                if (call.method == 'present') presents += 1;
                return true;
              });
          final actions = _RecordingReactionActions();
          final harness = await _pumpMessageActionsPopover(
            tester,
            message: _message(),
            prefs: await _mockPrefs(),
            createChannelActions: (_) => actions,
            overrides: [
              customEmojiPaletteProvider.overrideWith(
                _ReadyCustomEmojiPalette.new,
              ),
            ],
          );
          await tester.pumpAndSettle();

          // A new message shifts the timeline and rebuilds the long-pressed
          // row away.
          if (!unmountWhilePickerOpen) {
            harness.launcherMounted.value = false;
            await tester.pumpAndSettle();
          }
          await tester.tap(find.byKey(const ValueKey('quick-reaction-more')));
          await tester.pumpAndSettle();
          expect(presents, 1);
          if (unmountWhilePickerOpen) {
            harness.launcherMounted.value = false;
            await tester.pumpAndSettle();
          }

          await _sendNativeEmojiPicker(tester, 'selected', '🎉');
          await _sendNativeEmojiPicker(tester, 'dismissed');

          expect(actions.added, ['🎉']);
          expect(actions.targetAuthors, ['alice']);
          expect(tester.takeException(), isNull);
        } finally {
          debugDefaultTargetPlatformOverride = previousPlatform;
        }
      });
    }
  });
}
