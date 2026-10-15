import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

Future<void> tapStatusBar(WidgetTester tester) async {
  await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    'flutter/status_bar',
    const JSONMethodCodec().encodeMethodCall(
      const MethodCall('handleScrollToTop'),
    ),
    (_) {},
  );
  await tester.pumpAndSettle();
}

void main() {
  for (final reduced in [false, true]) {
    testWidgets(
      'status tap scrolls an explicit controller to top, reduced=$reduced',
      (tester) async {
        final controller = ScrollController();
        addTearDown(controller.dispose);
        await tester.pumpWidget(
          _testApp(
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(context).copyWith(disableAnimations: reduced),
              child: child!,
            ),
            home: FrostedScaffold(
              appBar: const FrostedAppBar(title: Text('Test')),
              body: ListView.builder(
                controller: controller,
                itemCount: 100,
                itemBuilder: (_, index) =>
                    SizedBox(height: 60, child: Text('Row $index')),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        controller.jumpTo(900);
        await tester.pumpAndSettle();
        await tapStatusBar(tester);
        expect(controller.offset, 0);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );
  }

  testWidgets(
    'status tap does not scroll a page under a modal',
    (tester) async {
      final controller = ScrollController();
      addTearDown(controller.dispose);
      late BuildContext pageContext;
      await tester.pumpWidget(
        _testApp(
          home: FrostedScaffold(
            appBar: const FrostedAppBar(title: Text('Test')),
            body: Builder(
              builder: (context) {
                pageContext = context;
                return ListView.builder(
                  controller: controller,
                  itemCount: 100,
                  itemBuilder: (_, index) =>
                      SizedBox(height: 60, child: Text('Row $index')),
                );
              },
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      controller.jumpTo(900);
      await tester.pumpAndSettle();
      showModalBottomSheet<void>(
        context: pageContext,
        builder: (_) => const SizedBox(height: 200),
      );
      await tester.pumpAndSettle();
      await tapStatusBar(tester);
      expect(controller.offset, 900);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );
  testWidgets(
    'status tap expands a pinned title and returns its list to top',
    (tester) async {
      await tester.pumpWidget(
        _testApp(
          home: FrostedScaffold(
            nativePinnedBody: true,
            appBar: const FrostedAppBar(
              title: Text('Title'),
              nativeLargeTitle: true,
            ),
            body: ListView.builder(
              itemCount: 100,
              itemBuilder: (_, index) =>
                  SizedBox(height: 60, child: Text('Pinned $index')),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.drag(find.byType(ListView), const Offset(0, -900));
      await tester.pumpAndSettle();
      final nested = tester.state<NestedScrollViewState>(
        find.byType(NestedScrollView),
      );
      expect(nested.innerController.offset, greaterThan(0));
      await tapStatusBar(tester);
      expect(nested.innerController.offset, 0);
      expect(nested.outerController.offset, 0);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets(
    'status tap leaves an inactive tab alone',
    (tester) async {
      final controllers = [ScrollController(), ScrollController()];
      addTearDown(() {
        for (final controller in controllers) {
          controller.dispose();
        }
      });
      await tester.pumpWidget(
        _testApp(
          home: IndexedStack(
            index: 1,
            children: [
              for (var index = 0; index < 2; index++)
                FrostedScaffold(
                  appBar: const FrostedAppBar(title: Text('Tab')),
                  body: ListView.builder(
                    controller: controllers[index],
                    itemCount: 100,
                    itemBuilder: (_, row) =>
                        SizedBox(height: 60, child: Text('Row $row')),
                  ),
                ),
            ],
          ),
        ),
      );
      await tester.pumpAndSettle();
      for (final controller in controllers) {
        controller.jumpTo(900);
      }
      await tester.pumpAndSettle();
      await tapStatusBar(tester);
      expect(controllers[0].offset, 900);
      expect(controllers[1].offset, 0);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );
}

Widget _testApp({Widget? home, TransitionBuilder? builder}) => ProviderScope(
  child: MaterialApp(home: home, builder: builder),
);
