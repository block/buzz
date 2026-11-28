import 'package:buzz/shared/widgets/orphaned_keyboard_guard.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_test/flutter_test.dart';

class _Guarded extends HookWidget {
  const _Guarded({required this.focus});

  final FocusNode focus;

  @override
  Widget build(BuildContext context) {
    useOrphanedKeyboardGuard();
    return MaterialApp(
      home: Scaffold(body: TextField(focusNode: focus)),
    );
  }
}

void main() {
  testWidgets('hides a keyboard restored after resume only when no field has '
      'focus', (tester) async {
    final hides = <int>[];
    var calls = 0;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.textInput,
      (call) async {
        calls++;
        if (call.method == 'TextInput.hide') hides.add(calls);
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.textInput,
        null,
      ),
    );
    addTearDown(tester.view.reset);
    final focus = FocusNode();
    addTearDown(focus.dispose);
    await tester.pumpWidget(_Guarded(focus: focus));

    Future<void> switchAway() async {
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      tester.view.viewInsets = FakeViewPadding.zero;
      await tester.pump();
    }

    Future<void> returnWithKeyboard({required bool insetAfterResume}) async {
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      if (!insetAfterResume) {
        tester.view.viewInsets = const FakeViewPadding(bottom: 900);
      }
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      if (insetAfterResume) {
        tester.view.viewInsets = const FakeViewPadding(bottom: 900);
      }
      await tester.pump();
    }

    // A focused field keeps its keyboard across the switch.
    focus.requestFocus();
    await tester.pump();
    await switchAway();
    await returnWithKeyboard(insetAfterResume: false);
    expect(hides, isEmpty);

    // UIKit restores the keyboard while Flutter has no field focused: hidden,
    // whether the inset is back before resume or arrives just after it.
    for (final insetAfterResume in [false, true]) {
      focus.unfocus();
      await switchAway();
      hides.clear();
      await returnWithKeyboard(insetAfterResume: insetAfterResume);
      expect(hides, hasLength(1), reason: 'insetAfterResume=$insetAfterResume');
    }

    // Outside the resume window the guard leaves keyboard changes alone.
    hides.clear();
    tester.view.viewInsets = FakeViewPadding.zero;
    await tester.pump(orphanedKeyboardResumeWindow);
    tester.view.viewInsets = const FakeViewPadding(bottom: 900);
    await tester.pump();
    expect(hides, isEmpty);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}
