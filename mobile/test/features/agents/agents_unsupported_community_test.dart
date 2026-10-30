import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  testWidgets(
    'a remote listing failure is hidden where remote agents are off',
    (tester) async {
      final h = await AgentsHarness.create(relayUrl: 'wss://relay.example');
      h.registry.failListing = true;
      await h.open(tester, directory: []);

      expect(h.controller.listingFailed, isTrue);
      expect(find.text('Could not load agents'), findsNothing);
      expect(find.text('You haven’t added any agents yet.'), findsOneWidget);
    },
  );
}
