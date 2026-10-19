import 'dart:async';

import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

const _request = (
  method: 'POST',
  url: 'https://relay.example/query',
  bodyBytes: <int>[91, 93],
  nsec: 'test-key',
);

void main() {
  test(
    'serializes workers and rejects saturation without retaining a job',
    () async {
      final worker = Completer<String>();
      var calls = 0;
      final signer = RelayQuerySigner(
        sign: (_) {
          calls++;
          return worker.future;
        },
      );
      final requests = List.generate(64, (_) => signer.sign(_request));
      await expectLater(signer.sign(_request), throwsStateError);
      expect(calls, 1);
      worker.complete('signed');
      expect(await Future.wait(requests), everyElement('signed'));
      expect(calls, 64);
      expect(await signer.sign(_request), 'signed');
    },
  );

  test(
    'retirement cancels queued work and rejects the active result',
    () async {
      final worker = Completer<String>();
      var calls = 0;
      final signer = RelayQuerySigner(
        sign: (_) {
          calls++;
          return worker.future;
        },
      );
      final requests = List.generate(
        3,
        (_) => expectLater(signer.sign(_request), throwsStateError),
      );
      await pumpEventQueue();
      expect(calls, 1);
      signer.retire();
      worker.complete('retired proof');
      await Future.wait(requests);
      expect(calls, 1);
      expect(await signer.sign(_request), 'retired proof');
      expect(calls, 2);
    },
  );

  test(
    'retirement releases queued capacity before the active worker ends',
    () async {
      final oldWorker = Completer<String>();
      final newWorker = Completer<String>();
      var calls = 0;
      final signer = RelayQuerySigner(
        sign: (_) {
          calls++;
          return calls == 1 ? oldWorker.future : newWorker.future;
        },
      );
      final active = expectLater(signer.sign(_request), throwsStateError);
      final queued = List.generate(
        63,
        (_) => expectLater(signer.sign(_request), throwsStateError),
      );
      await pumpEventQueue();
      signer.retire();
      final replacement = signer.sign(_request);
      var queuedSettled = false;
      unawaited(Future.wait(queued).then((_) => queuedSettled = true));
      await pumpEventQueue();
      expect(queuedSettled, isTrue);
      expect(calls, 1);
      oldWorker.complete('retired');
      await active;
      await pumpEventQueue();
      expect(calls, 2);
      newWorker.complete('replacement');
      expect(await replacement, 'replacement');
    },
  );

  test('a failed worker does not poison the next request', () async {
    var calls = 0;
    final signer = RelayQuerySigner(
      sign: (_) async {
        if (++calls == 1) throw const FormatException('invalid signing key');
        return 'signed';
      },
    );
    final failed = expectLater(signer.sign(_request), throwsFormatException);
    final next = signer.sign(_request);
    await failed;
    expect(await next, 'signed');
  });
}
