import 'package:buzz/features/pairing/pairing_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets(
    'welcome flight moves, pauses accessibly, and leaves actions usable',
    (tester) async {
      tester.view.physicalSize = const Size(402, 874);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final reducedMotion = ValueNotifier(false);
      addTearDown(reducedMotion.dispose);
      var communityLinks = 0;
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: ValueListenableBuilder<bool>(
            valueListenable: reducedMotion,
            builder: (context, reduced, _) => MediaQuery(
              data: MediaQuery.of(context).copyWith(disableAnimations: reduced),
              child: PairingPage(
                connectionLanding: true,
                onCommunityLink: () => communityLinks++,
              ),
            ),
          ),
        ),
      );
      Offset beePosition() {
        final transform = tester
            .widget<Transform>(find.byKey(const ValueKey('welcome-bee-0')))
            .transform;
        return Offset(transform.storage[12], transform.storage[13]);
      }

      final entrance = beePosition();
      for (var frame = 0; frame < 120; frame++) {
        await tester.pump(const Duration(milliseconds: 17));
      }
      expect(beePosition(), isNot(entrance));
      final flying = beePosition();
      await tester.pump(const Duration(milliseconds: 50));
      expect(beePosition(), isNot(flying));
      await tester.tap(find.text('Enter a community link').hitTestable());
      expect(communityLinks, 1);

      expect(find.byKey(const ValueKey('welcome-bee-11')), findsOneWidget);
      Size flightSize() =>
          tester.getSize(find.byKey(const Key('welcome-bee-flight-area')));
      final portraitSize = flightSize();
      final portrait = beePosition();
      tester.view.physicalSize = const Size(874, 402);
      await tester.pump();
      expect(
        beePosition().dx,
        closeTo(portrait.dx / portraitSize.width * flightSize().width, 0.001),
      );
      expect(
        beePosition().dy,
        closeTo(portrait.dy / portraitSize.height * flightSize().height, 0.001),
      );
      expect(find.byKey(const ValueKey('welcome-bee-11')), findsOneWidget);
      final landscapeSize = flightSize();
      final landscape = beePosition();
      await tester.pump(const Duration(milliseconds: 50));
      expect(beePosition(), isNot(landscape));
      await tester.ensureVisible(find.text('Enter a community link'));
      await tester.pump();
      await tester.tap(find.text('Enter a community link').hitTestable());
      expect(communityLinks, 2);
      final beforeReturn = beePosition();
      tester.view.physicalSize = const Size(402, 874);
      await tester.pump();
      expect(
        beePosition().dx,
        closeTo(
          beforeReturn.dx / landscapeSize.width * flightSize().width,
          0.001,
        ),
      );
      expect(
        beePosition().dy,
        closeTo(
          beforeReturn.dy / landscapeSize.height * flightSize().height,
          0.001,
        ),
      );

      reducedMotion.value = true;
      await tester.pump();
      final still = beePosition();
      await tester.pump(const Duration(seconds: 1));
      expect(beePosition(), still);
      expect(find.bySemanticsLabel('Buzz'), findsOneWidget);
      expect(find.bySemanticsLabel('Animated bee'), findsNothing);

      reducedMotion.value = false;
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      expect(beePosition(), isNot(still));
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      await tester.pump();
      final paused = beePosition();
      await tester.pump(const Duration(seconds: 1));
      expect(beePosition(), paused);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      expect(beePosition(), isNot(paused));
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
}
