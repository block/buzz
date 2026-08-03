import 'dart:async';

import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('uses the default for absent and non-positive hints', () {
    var now = DateTime.utc(2026);
    final timers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      now: () => now,
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );

    gate.activate(null);
    expect(gate.remainingMs(), 10000);
    expect(timers.single.duration, const Duration(seconds: 10));

    now = now.add(const Duration(seconds: 11));
    gate.activate(0);
    expect(timers.last.duration, const Duration(seconds: 10));
  });

  test('clamps large hints to five minutes', () {
    final timers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      now: () => DateTime.utc(2026),
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );

    gate.activate(999999);

    expect(timers.single.duration, const Duration(seconds: 300));
    expect(gate.remainingMs(), 300000);
  });

  test('overlapping activations only extend the active window', () async {
    var now = DateTime.utc(2026);
    final timers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      now: () => now,
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );

    gate.activate(30);
    final wait = gate.wait();
    final firstTimer = timers.single;

    now = now.add(const Duration(seconds: 5));
    gate.activate(10);
    expect(timers, hasLength(1));
    expect(gate.remainingMs(), 25000);

    gate.activate(40);
    expect(timers, hasLength(2));
    expect(firstTimer.isActive, isFalse);
    expect(gate.remainingMs(), 40000);

    timers.last.fire();
    await wait;
    expect(gate.isActive, isFalse);
    expect(gate.remainingMs(), 0);
  });

  test('reset releases waiters and cancels the active timer', () async {
    final timers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );

    gate.activate(20);
    final wait = gate.wait();
    gate.reset();

    await wait;
    expect(timers.single.isActive, isFalse);
    expect(gate.isActive, isFalse);
  });
}

class _ManualTimer implements Timer {
  _ManualTimer(this.duration, this._callback);

  final Duration duration;
  final void Function() _callback;
  bool _active = true;

  void fire() {
    if (!_active) return;
    _active = false;
    _callback();
  }

  @override
  void cancel() => _active = false;

  @override
  bool get isActive => _active;

  @override
  int get tick => _active ? 0 : 1;
}
