import 'dart:async';

import 'package:buzz/features/profile/camera_disposal_barrier.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('failed disposal settles before a replacement camera release', () async {
    final barrier = CameraDisposalBarrier();
    final firstDispose = Completer<void>();
    final secondDispose = Completer<void>();
    var secondStarted = false;

    final first = barrier.release(() => firstDispose.future);
    final second = barrier.release(() {
      secondStarted = true;
      return secondDispose.future;
    });

    expect(secondStarted, isFalse);
    firstDispose.completeError(StateError('native camera teardown failed'));
    await first;
    await Future<void>.delayed(Duration.zero);
    expect(secondStarted, isTrue);

    secondDispose.complete();
    await second;
    await barrier.settled;
  });

  test(
    'serializes replacement releases without overlapping ownership',
    () async {
      final barrier = CameraDisposalBarrier();
      final firstDispose = Completer<void>();
      var activeDisposals = 0;
      var overlapped = false;

      final first = barrier.release(() async {
        activeDisposals++;
        if (activeDisposals > 1) overlapped = true;
        await firstDispose.future;
        activeDisposals--;
      });
      final second = barrier.release(() async {
        activeDisposals++;
        if (activeDisposals > 1) overlapped = true;
        activeDisposals--;
      });

      await Future<void>.delayed(Duration.zero);
      expect(activeDisposals, 1);
      firstDispose.complete();
      await Future.wait([first, second]);

      expect(overlapped, isFalse);
    },
  );
}
