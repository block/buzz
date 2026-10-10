import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/read_state/read_marker_clamp.dart';

void main() {
  const now = 1750000000;

  group('clampReadMarker', () {
    test('leaves a past timestamp untouched', () {
      expect(clampReadMarker(now - 3600, nowSeconds: now), now - 3600);
    });

    test('leaves ordinary clock drift untouched', () {
      // A few seconds of NTP drift is normal and must not be mangled — the
      // marker has to stay byte-identical or it stops matching the message it
      // was derived from.
      expect(clampReadMarker(now + 5, nowSeconds: now), now + 5);
    });

    test('accepts exactly the relay ingest bound', () {
      // ingest.rs admits +900 inclusive, so this value can legitimately reach a
      // client and must not be treated as poison.
      expect(
        clampReadMarker(now + readMarkerMaxSkewSeconds, nowSeconds: now),
        now + readMarkerMaxSkewSeconds,
      );
    });

    test('repairs one second past the bound', () {
      expect(
        clampReadMarker(now + readMarkerMaxSkewSeconds + 1, nowSeconds: now),
        now,
      );
    });

    test('repairs a year-ahead marker to now rather than to the bound', () {
      // Repairing to `now + tolerance` would itself be a future frontier and
      // would keep suppressing messages for the width of the tolerance.
      final repaired = clampReadMarker(now + 31536000, nowSeconds: now);
      expect(repaired, now);
      expect(repaired, lessThan(now + readMarkerMaxSkewSeconds));
    });

    test('uses 900, matching the relay ingest gate and not the 120s '
        'moderation replay window', () {
      expect(readMarkerMaxSkewSeconds, 900);
    });
  });

  group('clampReadMarkers', () {
    test('repairs values without ever pruning a key', () {
      // The load-bearing property: a device whose own clock is wrong would
      // otherwise judge every stored marker implausible. Dropping them would
      // destroy read state irrecoverably; clamping cannot.
      final contexts = <String, int>{
        'channel-a': now - 100,
        'channel-b': now + 31536000,
        'msg:abc': now + 10,
        'thread:def': now + readMarkerMaxSkewSeconds + 1,
      };

      final clamped = clampReadMarkers(contexts, nowSeconds: now);

      expect(clamped.keys.toSet(), contexts.keys.toSet());
      expect(clamped['channel-a'], now - 100);
      expect(clamped['channel-b'], now);
      expect(clamped['msg:abc'], now + 10);
      expect(clamped['thread:def'], now);
    });

    test('never raises a marker', () {
      final contexts = <String, int>{
        'a': now - 5,
        'b': now + 5,
        'c': now + 999999,
      };
      final clamped = clampReadMarkers(contexts, nowSeconds: now);
      for (final entry in contexts.entries) {
        expect(
          clamped[entry.key]! <= entry.value,
          isTrue,
          reason:
              'clamping must only ever lower a marker, so it can only widen '
              'what counts as unread — never swallow a reply',
        );
      }
    });

    test('preserves an empty map', () {
      expect(clampReadMarkers(<String, int>{}, nowSeconds: now), isEmpty);
    });
  });
}
