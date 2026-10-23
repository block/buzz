import 'dart:io';
import 'package:buzz/shared/crypto/event_signer.dart';
import 'package:flutter/material.dart';
import 'package:nostr/nostr.dart' as nostr;

// Run on iOS with the same event construction on the UI isolate for both paths.
void main() {
  WidgetsFlutterBinding.ensureInitialized();
  try {
    if (!Platform.isIOS) throw StateError('Requires iOS');
    final results = <String>[];
    final key = '0' * 63 + '3';
    final publicKey = nostr.Keys(key).public;
    nostr.Event create(bool native) {
      final tags = [
        ['t', 'get'],
        ['expiration', '1800000060'],
      ];
      return native
          ? signEvent(
              kind: 24242,
              content: 'Get buzz-media',
              tags: tags,
              createdAt: 1800000000,
              secretKey: key,
              pubkey: publicKey,
            )
          : nostr.Event.from(
              kind: 24242,
              content: 'Get buzz-media',
              tags: tags,
              createdAt: 1800000000,
              secretKey: key,
              pubkey: publicKey,
            );
    }

    for (final native in [false, true]) {
      final watch = Stopwatch()..start();
      final event = create(native);
      watch.stop();
      results.add(
        'first measured ${native ? "native" : "Dart"}: ${watch.elapsedMicroseconds} us',
      );
      if (!event.isValid()) throw StateError('Invalid signature');
    }
    if (create(false).id != create(true).id) {
      throw StateError('Different events');
    }
    for (var warm = 0; warm < 10; warm++) {
      create(false);
      create(true);
    }
    for (final count in [1, 8, 32, 64]) {
      final samples = {false: <int>[], true: <int>[]};
      for (var round = 0; round < 7; round++) {
        for (final native in round.isEven ? [false, true] : [true, false]) {
          final watch = Stopwatch()..start();
          for (var i = 0; i < count; i++) {
            create(native);
          }
          watch.stop();
          samples[native]!.add(watch.elapsedMicroseconds);
        }
      }
      for (final native in [false, true]) {
        final values = samples[native]!..sort();
        results.add(
          'burst $count ${native ? "native" : "Dart"}: '
          'median ${values[3]} min ${values.first} max ${values.last} us',
        );
      }
    }
    for (final result in results) {
      debugPrint('SIGNING_PROBE $result');
    }
    debugPrint('SIGNING_PROBE COMPLETE');
    runApp(
      MaterialApp(
        home: Scaffold(body: SafeArea(child: Text(results.join('\n')))),
      ),
    );
  } catch (error) {
    debugPrint('SIGNING_PROBE FAILED: $error');
    rethrow;
  }
}
