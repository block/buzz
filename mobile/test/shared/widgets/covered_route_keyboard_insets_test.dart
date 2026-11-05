import 'package:buzz/shared/widgets/covered_route_keyboard_insets.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_test/flutter_test.dart';

/// Counts builds and records the insets the page saw last.
class _Probe extends StatelessWidget {
  const _Probe(this.builds, this.seen);

  final List<int> builds;
  final List<EdgeInsets> seen;

  @override
  Widget build(BuildContext context) {
    builds[0]++;
    seen.add(MediaQuery.viewInsetsOf(context));
    MediaQuery.paddingOf(context);
    return const SizedBox.expand();
  }
}

void main() {
  testWidgets('a covered page ignores keyboard motion and gets the live '
      'insets back when it is current again', (tester) async {
    final builds = [0];
    final seen = <EdgeInsets>[];
    final navigator = GlobalKey<NavigatorState>();
    var insets = EdgeInsets.zero;
    late StateSetter setOuter;
    // Built once, so only the inherited MediaQuery changes below.
    final app = MaterialApp(
      navigatorKey: navigator,
      home: CoveredRouteKeyboardInsets(child: _Probe(builds, seen)),
    );

    await tester.pumpWidget(
      StatefulBuilder(
        builder: (context, setState) {
          setOuter = setState;
          return MediaQuery(
            data: MediaQueryData(
              size: const Size(400, 800),
              viewInsets: insets,
              padding: EdgeInsets.only(bottom: insets.bottom > 0 ? 0 : 34),
            ),
            child: app,
          );
        },
      ),
    );
    expect(seen.last, EdgeInsets.zero);

    // A transparent route above, as the native member picker uses.
    navigator.currentState!.push(
      PageRouteBuilder<void>(
        opaque: false,
        transitionDuration: Duration.zero,
        pageBuilder: (_, _, _) => const SizedBox(),
      ),
    );
    await tester.pumpAndSettle();
    final coveredBuilds = builds[0];

    // Keyboard frames for the covering route's field.
    for (final bottom in [80.0, 160.0, 240.0, 300.0]) {
      setOuter(() => insets = EdgeInsets.only(bottom: bottom));
      await tester.pump();
    }
    expect(builds[0], coveredBuilds);
    expect(seen.last, EdgeInsets.zero);

    // Uncovered while the covering route's keyboard is still up: the page
    // stays where it was instead of jumping up, then takes live values once
    // the keyboard is gone.
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(seen.last, EdgeInsets.zero);
    for (final bottom in [200.0, 100.0]) {
      setOuter(() => insets = EdgeInsets.only(bottom: bottom));
      await tester.pump();
      expect(seen.last, EdgeInsets.zero);
    }
    setOuter(() => insets = EdgeInsets.zero);
    await tester.pump();
    expect(seen.last, EdgeInsets.zero);
    setOuter(() => insets = const EdgeInsets.only(bottom: 120));
    await tester.pump();
    expect(seen.last, const EdgeInsets.only(bottom: 120));
  });

  testWidgets('an uncovered page never rises above the live keyboard and '
      'follows it once it reaches the held value', (tester) async {
    final builds = [0];
    final seen = <EdgeInsets>[];
    final navigator = GlobalKey<NavigatorState>();
    var insets = const EdgeInsets.only(bottom: 300);
    late StateSetter setOuter;
    final app = MaterialApp(
      navigatorKey: navigator,
      home: CoveredRouteKeyboardInsets(child: _Probe(builds, seen)),
    );

    await tester.pumpWidget(
      StatefulBuilder(
        builder: (context, setState) {
          setOuter = setState;
          return MediaQuery(
            data: MediaQueryData(
              size: const Size(400, 800),
              viewInsets: insets,
            ),
            child: app,
          );
        },
      ),
    );

    navigator.currentState!.push(
      PageRouteBuilder<void>(
        opaque: false,
        transitionDuration: Duration.zero,
        pageBuilder: (_, _, _) => const SizedBox(),
      ),
    );
    await tester.pumpAndSettle();
    // The page's keyboard closes to 100 behind the covering route, whose own
    // field then raises the keyboard back to 300.
    for (final bottom in [200.0, 100.0, 200.0, 300.0]) {
      setOuter(() => insets = EdgeInsets.only(bottom: bottom));
      await tester.pump();
    }
    expect(seen.last, const EdgeInsets.only(bottom: 100));

    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(seen.last, const EdgeInsets.only(bottom: 100));
    for (final bottom in [250.0, 150.0, 100.0]) {
      setOuter(() => insets = EdgeInsets.only(bottom: bottom));
      await tester.pump();
      expect(seen.last, const EdgeInsets.only(bottom: 100));
    }
    setOuter(() => insets = const EdgeInsets.only(bottom: 40));
    await tester.pump();
    expect(seen.last, const EdgeInsets.only(bottom: 40));
  });

  testWidgets('an uncovered page follows the keyboard its own field raises', (
    tester,
  ) async {
    final builds = [0];
    final seen = <EdgeInsets>[];
    final navigator = GlobalKey<NavigatorState>();
    final focusNode = FocusNode();
    addTearDown(focusNode.dispose);
    var insets = EdgeInsets.zero;
    late StateSetter setOuter;
    final app = MaterialApp(
      navigatorKey: navigator,
      home: CoveredRouteKeyboardInsets(
        child: Scaffold(
          resizeToAvoidBottomInset: false,
          body: Column(
            children: [
              Expanded(child: _Probe(builds, seen)),
              TextField(focusNode: focusNode),
            ],
          ),
        ),
      ),
    );

    await tester.pumpWidget(
      StatefulBuilder(
        builder: (context, setState) {
          setOuter = setState;
          return MediaQuery(
            data: MediaQueryData(
              size: const Size(400, 800),
              viewInsets: insets,
            ),
            child: app,
          );
        },
      ),
    );

    navigator.currentState!.push(
      PageRouteBuilder<void>(
        opaque: false,
        transitionDuration: Duration.zero,
        pageBuilder: (_, _, _) => const SizedBox(),
      ),
    );
    await tester.pumpAndSettle();
    setOuter(() => insets = const EdgeInsets.only(bottom: 300));
    await tester.pump();
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(seen.last, EdgeInsets.zero);

    // The page's own field takes focus while the keyboard is still up, so the
    // page has to follow the keyboard rather than hold below it.
    focusNode.requestFocus();
    await tester.pump();
    await tester.pump();
    expect(focusNode.hasPrimaryFocus, isTrue);
    expect(seen.last, const EdgeInsets.only(bottom: 300));
    setOuter(() => insets = const EdgeInsets.only(bottom: 280));
    await tester.pump();
    expect(seen.last, const EdgeInsets.only(bottom: 280));
  });

  testWidgets('a covered page follows its own keyboard closing', (
    tester,
  ) async {
    final builds = [0];
    final seen = <EdgeInsets>[];
    final navigator = GlobalKey<NavigatorState>();
    var insets = const EdgeInsets.only(bottom: 300);
    late StateSetter setOuter;
    final app = MaterialApp(
      navigatorKey: navigator,
      home: CoveredRouteKeyboardInsets(child: _Probe(builds, seen)),
    );

    await tester.pumpWidget(
      StatefulBuilder(
        builder: (context, setState) {
          setOuter = setState;
          return MediaQuery(
            data: MediaQueryData(
              size: const Size(400, 800),
              viewInsets: insets,
              padding: EdgeInsets.only(bottom: insets.bottom > 0 ? 0 : 34),
            ),
            child: app,
          );
        },
      ),
    );
    expect(seen.last, const EdgeInsets.only(bottom: 300));

    // A sheet takes focus, so the page's keyboard closes behind it.
    navigator.currentState!.push(
      PageRouteBuilder<void>(
        opaque: false,
        transitionDuration: Duration.zero,
        pageBuilder: (_, _, _) => const SizedBox(),
      ),
    );
    await tester.pumpAndSettle();
    for (final bottom in [200.0, 100.0, 0.0]) {
      setOuter(() => insets = EdgeInsets.only(bottom: bottom));
      await tester.pump();
      expect(seen.last, EdgeInsets.only(bottom: bottom));
    }

    // A keyboard the covering route raises is still ignored.
    final coveredBuilds = builds[0];
    for (final bottom in [80.0, 160.0, 300.0]) {
      setOuter(() => insets = EdgeInsets.only(bottom: bottom));
      await tester.pump();
    }
    expect(builds[0], coveredBuilds);
    expect(seen.last, EdgeInsets.zero);
  });

  testWidgets('a covered page takes the new padding when rotated', (
    tester,
  ) async {
    EdgeInsets? padding;
    final navigator = GlobalKey<NavigatorState>();
    var landscape = false;
    var keyboard = 0.0;
    late StateSetter setOuter;
    final app = MaterialApp(
      navigatorKey: navigator,
      home: CoveredRouteKeyboardInsets(
        child: Builder(
          builder: (context) {
            padding = MediaQuery.paddingOf(context);
            return const SizedBox.expand();
          },
        ),
      ),
    );

    await tester.pumpWidget(
      StatefulBuilder(
        builder: (context, setState) {
          setOuter = setState;
          return MediaQuery(
            data: MediaQueryData(
              size: landscape ? const Size(800, 400) : const Size(400, 800),
              padding: landscape
                  ? const EdgeInsets.symmetric(horizontal: 44)
                  : const EdgeInsets.only(top: 44, bottom: 34),
              viewInsets: EdgeInsets.only(bottom: keyboard),
            ),
            child: app,
          );
        },
      ),
    );
    expect(padding, const EdgeInsets.only(top: 44, bottom: 34));

    navigator.currentState!.push(
      PageRouteBuilder<void>(
        opaque: false,
        transitionDuration: Duration.zero,
        pageBuilder: (_, _, _) => const SizedBox(),
      ),
    );
    await tester.pumpAndSettle();

    // The covering route raises its own keyboard, so only the size change can
    // release the held portrait padding.
    setOuter(() => keyboard = 300);
    await tester.pump();
    expect(padding, const EdgeInsets.only(top: 44, bottom: 34));

    setOuter(() => landscape = true);
    await tester.pump();
    expect(padding, const EdgeInsets.symmetric(horizontal: 44));
  });

  testWidgets('a current page follows the keyboard', (tester) async {
    final builds = [0];
    final seen = <EdgeInsets>[];
    await tester.pumpWidget(
      MediaQuery(
        data: const MediaQueryData(size: Size(400, 800)),
        child: MaterialApp(
          home: CoveredRouteKeyboardInsets(child: _Probe(builds, seen)),
        ),
      ),
    );
    await tester.pumpWidget(
      MediaQuery(
        data: const MediaQueryData(
          size: Size(400, 800),
          viewInsets: EdgeInsets.only(bottom: 300),
        ),
        child: MaterialApp(
          home: CoveredRouteKeyboardInsets(child: _Probe(builds, seen)),
        ),
      ),
    );
    expect(seen.last, const EdgeInsets.only(bottom: 300));
  });

  testWidgets('covering the page keeps its state', (tester) async {
    final navigator = GlobalKey<NavigatorState>();
    final mounts = <int>[0];
    await tester.pumpWidget(
      MaterialApp(
        navigatorKey: navigator,
        home: CoveredRouteKeyboardInsets(child: _MountCounter(mounts)),
      ),
    );
    expect(mounts.single, 1);

    navigator.currentState!.push(
      PageRouteBuilder<void>(
        opaque: false,
        transitionDuration: Duration.zero,
        pageBuilder: (_, _, _) => const SizedBox(),
      ),
    );
    await tester.pumpAndSettle();
    navigator.currentState!.pop();
    await tester.pumpAndSettle();

    expect(mounts.single, 1);
  });
}

class _MountCounter extends HookWidget {
  const _MountCounter(this.mounts);

  final List<int> mounts;

  @override
  Widget build(BuildContext context) {
    useEffect(() {
      mounts[0]++;
      return null;
    }, const []);
    return const SizedBox();
  }
}
