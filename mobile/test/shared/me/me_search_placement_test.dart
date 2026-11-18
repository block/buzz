import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/me/me_search_placement.dart';

void main() {
  for (final error in [
    const FormatException('malformed'),
    StateError('unavailable'),
  ]) {
    test('ordinary search remains navigable when Me is $error', () async {
      final container = ProviderContainer(
        overrides: [
          meGroupsProvider.overrideWith((ref) => Stream.error(error)),
        ],
        retry: (_, _) => null,
      );
      addTearDown(container.dispose);
      expect(
        await container.read(meSearchPlacementProvider('ordinary').future),
        isFalse,
      );
    });
  }
  testWidgets('unresponsive Me lookup has a bounded ordinary fallback', (
    tester,
  ) async {
    final stream = StreamController<MeGroups>();
    final container = ProviderContainer(
      overrides: [meGroupsProvider.overrideWith((ref) => stream.stream)],
    );
    addTearDown(() async {
      container.dispose();
      await stream.close();
    });
    final result = container.read(meSearchPlacementProvider('ordinary').future);
    await tester.pump();
    await tester.pump(const Duration(seconds: 2));
    expect(await result, isFalse);
  });
  test(
    'cached personal placement is used without waiting for refresh',
    () async {
      final container = ProviderContainer(
        overrides: [
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(const MeGroups(channels: ['personal'])),
          ),
        ],
      );
      addTearDown(container.dispose);
      container.listen(meGroupsProvider, (_, _) {});
      await container.read(meGroupsProvider.future);
      expect(
        await container.read(meSearchPlacementProvider('personal').future),
        isTrue,
      );
      expect(
        await container.read(meSearchPlacementProvider('ordinary').future),
        isFalse,
      );
    },
  );
}
