import 'dart:async';

import 'package:buzz/shared/hosted/use_known_communities.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('discovery refreshes, retains results on error and pauses', (
    tester,
  ) async {
    var calls = 0;
    var active = true;
    var fail = false;
    Future<bool> load() async {
      calls++;
      if (fail) throw StateError('offline');
      return calls > 1;
    }

    Widget page() => MaterialApp(
      home: HookBuilder(
        builder: (_) {
          final found = useKnownCommunities(
            accountKey: 'a',
            active: active,
            load: load,
          );
          return Text(found ? 'found' : 'empty');
        },
      ),
    );
    await tester.pumpWidget(page());
    await tester.pump();
    expect(calls, 1);
    expect(find.text('empty'), findsOneWidget);
    await tester.pump(const Duration(seconds: 30));
    await tester.pump();
    expect(find.text('found'), findsOneWidget);
    fail = true;
    await tester.pump(const Duration(seconds: 30));
    await tester.pump();
    expect(find.text('found'), findsOneWidget);
    active = false;
    await tester.pumpWidget(page());
    final pausedCalls = calls;
    await tester.pump(const Duration(minutes: 1));
    expect(calls, pausedCalls);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('account change discards old results and in-flight responses', (
    tester,
  ) async {
    var account = 'a';
    final first = Completer<bool>();
    Widget page() => MaterialApp(
      home: HookBuilder(
        builder: (_) {
          final found = useKnownCommunities(
            accountKey: account,
            active: true,
            load: () => account == 'a' ? first.future : Future.value(false),
          );
          return Text(found ? 'found' : 'empty');
        },
      ),
    );
    await tester.pumpWidget(page());
    await tester.pump(const Duration(seconds: 30));
    account = 'b';
    await tester.pumpWidget(page());
    await tester.pump();
    first.complete(true);
    await tester.pump();
    expect(find.text('empty'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('backgrounding stops requests and resuming refreshes', (
    tester,
  ) async {
    var calls = 0;
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pumpWidget(
      MaterialApp(
        home: HookBuilder(
          builder: (_) {
            useKnownCommunities(
              accountKey: 'a',
              active: true,
              load: () async {
                calls++;
                return true;
              },
            );
            return const SizedBox();
          },
        ),
      ),
    );
    await tester.pump();
    expect(calls, 1);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    await tester.pump();
    await tester.pump(const Duration(minutes: 1));
    expect(calls, 1);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();
    expect(calls, 2);
    await tester.pumpWidget(const SizedBox());
  });
}
