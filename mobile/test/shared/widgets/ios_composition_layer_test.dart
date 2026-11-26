import 'package:buzz/shared/widgets/bottom_up_page_route.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('sheet backing never interrupts keyboard traversal', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      (_) async => null,
    );
    addTearDown(
      () => messenger.setMockMethodCallHandler(
        SystemChannels.platform_views,
        null,
      ),
    );
    final navigator = GlobalKey<NavigatorState>();
    final first = FocusNode();
    final second = FocusNode();
    addTearDown(first.dispose);
    addTearDown(second.dispose);
    await tester.pumpWidget(
      MaterialApp(navigatorKey: navigator, home: const Scaffold()),
    );
    navigator.currentState!.push(
      BottomUpPageRoute<void>(
        reducedMotion: false,
        builder: (_) => Scaffold(
          body: Column(
            children: [
              TextButton(
                focusNode: first,
                onPressed: () {},
                child: const Text('First'),
              ),
              TextButton(
                focusNode: second,
                onPressed: () {},
                child: const Text('Second'),
              ),
            ],
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    first.requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.pump();
    expect(second.hasFocus, isTrue);
    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.pump();
    expect(
      first.hasFocus,
      isTrue,
      reason: 'wrap directly to visible controls instead of the backing',
    );
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });
}
