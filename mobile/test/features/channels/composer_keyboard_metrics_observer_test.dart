import 'package:flutter_test/flutter_test.dart';

import 'package:buzz/features/channels/compose_bar.dart';

/// The composer only mounts its `TextField` while expanded, and it collapses
/// when the keyboard hides. Tapping the keyboard's voice-typing key swaps the
/// system keyboard for the speech IME, and the bottom inset briefly hits zero
/// mid-swap — so a naive "inset == 0 means hidden" check tore the composer down
/// before the speech IME could attach, and dictation silently did nothing.
void main() {
  testWidgets('ignores a transient inset drop while an IME swap is in flight', (
    tester,
  ) async {
    var hiddenCount = 0;
    final observer = ComposerKeyboardMetricsObserver(
      view: tester.view,
      onKeyboardHidden: () => hiddenCount++,
    );
    addTearDown(observer.dispose);
    addTearDown(tester.view.resetViewInsets);

    tester.view.viewInsets = const FakeViewPadding(bottom: 800);
    observer.didChangeMetrics();

    // The old IME detaches: inset collapses to zero...
    tester.view.viewInsets = FakeViewPadding.zero;
    observer.didChangeMetrics();
    await tester.pump(const Duration(milliseconds: 100));
    expect(
      hiddenCount,
      0,
      reason: 'must not tear the composer down before the swap settles',
    );

    // ...and the speech IME attaches before the settle delay elapses.
    tester.view.viewInsets = const FakeViewPadding(bottom: 800);
    observer.didChangeMetrics();
    await tester.pump(const Duration(milliseconds: 500));

    expect(hiddenCount, 0);
  });

  testWidgets('still reports a keyboard that stays hidden', (tester) async {
    var hiddenCount = 0;
    final observer = ComposerKeyboardMetricsObserver(
      view: tester.view,
      onKeyboardHidden: () => hiddenCount++,
    );
    addTearDown(observer.dispose);
    addTearDown(tester.view.resetViewInsets);

    tester.view.viewInsets = const FakeViewPadding(bottom: 800);
    observer.didChangeMetrics();

    tester.view.viewInsets = FakeViewPadding.zero;
    observer.didChangeMetrics();
    await tester.pump(const Duration(milliseconds: 500));

    expect(hiddenCount, 1);
  });

  testWidgets('a disposed observer never fires', (tester) async {
    var hiddenCount = 0;
    final observer = ComposerKeyboardMetricsObserver(
      view: tester.view,
      onKeyboardHidden: () => hiddenCount++,
    );
    addTearDown(tester.view.resetViewInsets);

    tester.view.viewInsets = const FakeViewPadding(bottom: 800);
    observer.didChangeMetrics();
    tester.view.viewInsets = FakeViewPadding.zero;
    observer.didChangeMetrics();

    observer.dispose();
    await tester.pump(const Duration(milliseconds: 500));

    expect(hiddenCount, 0);
  });
}
