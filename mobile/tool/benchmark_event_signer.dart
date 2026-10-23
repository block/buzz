import 'dart:ffi';
import 'dart:io';
import 'package:buzz/shared/crypto/event_signer.dart';

/// Host FFI benchmark. Pass the compiled adapter dylib path as the argument.
void main(List<String> args) {
  final setup = Stopwatch()..start();
  final signer = NativeEventSigner(DynamicLibrary.open(args.single));
  final key = '0' * 63 + '3';
  void sign() => signer.sign(
    kind: 24242,
    content: '',
    secretKey: key,
    tags: [
      ['t', 'get'],
      ['expiration', '1800000060'],
    ],
  );
  sign();
  stdout.writeln(
    'cold binding + context + complete event: ${setup.elapsedMicroseconds} us',
  );
  for (var i = 0; i < 100; i++) {
    sign();
  }
  for (final burst in [1, 8, 32, 64]) {
    final samples = <int>[];
    for (var round = 0; round < 100; round++) {
      final watch = Stopwatch()..start();
      for (var i = 0; i < burst; i++) {
        sign();
      }
      samples.add(watch.elapsedMicroseconds);
    }
    samples.sort();
    stdout.writeln(
      'burst $burst: median ${samples[50]} us, p95 ${samples[95]} us, max ${samples.last} us',
    );
  }
}
