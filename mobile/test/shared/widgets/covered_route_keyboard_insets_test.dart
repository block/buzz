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

    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(seen.last, const EdgeInsets.only(bottom: 300));
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
