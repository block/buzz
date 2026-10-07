import 'package:buzz/features/channels/message_action_backdrop_state.dart';
import 'package:buzz/features/channels/jump_to_latest_button.dart';
import 'package:buzz/features/channels/jump_to_latest_switcher.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

void main() {
  test(
    'keeps channel Latest hidden while followed-tail geometry catches up',
    () {
      expect(
        shouldHideChannelJumpToLatest(
          isAtLatest: false,
          followsLatest: true,
          userHasDetached: false,
        ),
        isTrue,
      );
      expect(
        shouldHideChannelJumpToLatest(
          isAtLatest: false,
          followsLatest: true,
          userHasDetached: true,
        ),
        isFalse,
        reason: 'A deliberate scroll away must still expose Latest.',
      );
      expect(
        shouldHideChannelJumpToLatest(
          isAtLatest: true,
          followsLatest: false,
          userHasDetached: true,
        ),
        isTrue,
        reason: 'Settled tail geometry remains authoritative.',
      );
    },
  );

  testWidgets('uses native iOS liquid glass outside message-action backdrops', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    const channel = MethodChannel('buzz/jump_to_latest_glass/41');
    final methodCalls = <MethodCall>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      methodCalls.add(call);
      return null;
    });
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(body: JumpToLatestButton(onPressed: () {})),
        ),
      );

      final nativeView = tester.widget<UiKitView>(find.byType(UiKitView));
      expect(nativeView.viewType, 'buzz/jump_to_latest_glass');
      expect(nativeView.creationParams, <String, Object>{
        'brightness': 'light',
      });
      expect(
        find.byKey(const ValueKey('channel-jump-to-latest-ios-glass')),
        findsOneWidget,
      );
      expect(
        tester.getSize(
          find.byKey(const ValueKey('channel-jump-to-latest-surface')),
        ),
        const Size.square(Grid.xl),
      );

      nativeView.onPlatformViewCreated!(41);
      await tester.pump();
      expect(
        methodCalls
            .lastWhere((call) => call.method == 'setBrightness')
            .arguments,
        'light',
      );

      messageActionBackdropActive.value = true;
      await tester.pump();
      expect(find.byType(UiKitView), findsNothing);
      expect(find.byType(BackdropFilter), findsOneWidget);
      expect(find.byIcon(LucideIcons.arrowDown), findsOneWidget);
    } finally {
      messageActionBackdropActive.value = false;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      );
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('keeps the composable Flutter glass control on Android', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    var pressed = false;
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Center(
              child: JumpToLatestButton(onPressed: () => pressed = true),
            ),
          ),
        ),
      );

      expect(find.byType(UiKitView), findsNothing);
      expect(find.byType(BackdropFilter), findsOneWidget);
      expect(find.byIcon(LucideIcons.arrowDown), findsOneWidget);
      expect(
        tester.getSize(find.byType(JumpToLatestButton)),
        const Size.square(Grid.xl),
      );
      expect(
        tester.getSize(
          find.byKey(const ValueKey('channel-jump-to-latest-surface')),
        ),
        const Size.square(Grid.lg),
      );

      await tester.tap(find.byType(JumpToLatestButton));
      expect(pressed, isTrue);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('mounts and removes native iOS Latest without animating it', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    var visible = true;
    late StateSetter update;
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: StatefulBuilder(
              builder: (context, setState) {
                update = setState;
                return JumpToLatestSwitcher(
                  id: 'thread',
                  visible: visible,
                  onPressed: () {},
                );
              },
            ),
          ),
        ),
      );

      expect(find.byType(AnimatedSwitcher), findsNothing);
      expect(find.byType(UiKitView), findsOneWidget);

      update(() => visible = false);
      await tester.pump();

      expect(find.byType(UiKitView), findsNothing);
      expect(
        find.byKey(const ValueKey('thread-jump-to-latest-hidden')),
        findsOneWidget,
      );
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('unmounts native iOS Latest while its route is inactive', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Builder(
            builder: (context) => Scaffold(
              body: Column(
                children: [
                  JumpToLatestButton(onPressed: () {}),
                  TextButton(
                    onPressed: () => Navigator.of(context).push<void>(
                      MaterialPageRoute<void>(
                        builder: (_) => const Scaffold(body: Text('Thread')),
                      ),
                    ),
                    child: const Text('Open thread'),
                  ),
                ],
              ),
            ),
          ),
        ),
      );

      expect(find.byType(UiKitView, skipOffstage: false), findsOneWidget);

      await tester.tap(find.text('Open thread'));
      await tester.pumpAndSettle();

      expect(find.text('Thread'), findsOneWidget);
      expect(
        find.byType(UiKitView, skipOffstage: false),
        findsNothing,
        reason: 'An underlying route must not keep a native glass layer alive.',
      );

      Navigator.of(tester.element(find.text('Thread'))).pop();
      await tester.pumpAndSettle();

      expect(find.byType(UiKitView, skipOffstage: false), findsOneWidget);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('gives thread controls thread-specific measurement keys', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: JumpToLatestSwitcher(
            id: 'thread',
            visible: true,
            onPressed: () {},
          ),
        ),
      ),
    );

    expect(find.byKey(const ValueKey('thread-jump-to-latest')), findsOneWidget);
    expect(
      find.byKey(const ValueKey('thread-jump-to-latest-surface')),
      findsOneWidget,
    );
    expect(find.text('Latest'), findsNothing);
  });
}
