part of '../activity_page_test.dart';

void _activityTabJourneys(
  Future<Widget> Function({
    Set<String> knownAgentPubkeys,
    TextScaler? textScaler,
  })
  buildTestable,
) {
  testWidgets('Activity type and sender menus work at phone width', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(390, 844));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    const directory = String.fromEnvironment('ACTIVITY_CAPTURE_DIR');
    if (directory.isNotEmpty) {
      await tester.runAsync(() async {
        for (final font in {
          'Inter': 'assets/fonts/InterVariable.ttf',
          'BuzzTabler': 'assets/fonts/TablerIcons.ttf',
        }.entries) {
          await (FontLoader(
            font.key,
          )..addFont(rootBundle.load(font.value))).load();
        }
      });
    }
    final capture = GlobalKey();
    await tester.pumpWidget(
      RepaintBoundary(
        key: capture,
        child: await buildTestable(knownAgentPubkeys: const {'bob_pk'}),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Activity'), findsOneWidget);
    expect(find.byKey(const ValueKey('activity-filter-menu')), findsNothing);
    expect(find.text('Anyone'), findsOneWidget);
    expect(find.text('All'), findsOneWidget);
    expect(find.text('Unread'), findsOneWidget);
    expect(find.text('Threads'), findsNothing);
    final typeSize = tester.getSize(
      find.byKey(const ValueKey('activity-type-menu')),
    );
    for (final key in [
      'activity-type-menu',
      'activity-sender-menu',
      'activity-unread-filter',
    ]) {
      final button = find.byKey(ValueKey(key));
      expect(tester.getSize(button).height, typeSize.height);
      expect(
        tester.widget<TextButton>(button).style!.padding!.resolve({}),
        const EdgeInsets.symmetric(horizontal: 12),
      );
      expect(
        tester.widget<TextButton>(button).style!.shape!.resolve({}),
        isA<StadiumBorder>(),
      );
    }

    expect(tester.takeException(), isNull);
    if (directory.isNotEmpty) {
      await tester.runAsync(() async {
        final boundary =
            capture.currentContext!.findRenderObject()!
                as RenderRepaintBoundary;
        final image = await boundary.toImage(pixelRatio: 2);
        final data = await image.toByteData(format: ui.ImageByteFormat.png);
        await Directory(directory).create(recursive: true);
        await File(
          '$directory/activity.png',
        ).writeAsBytes(data!.buffer.asUint8List());
        image.dispose();
      });
    }
    await tester.ensureVisible(
      find.byKey(const ValueKey('activity-sender-menu')),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('activity-sender-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Agents').last);
    await tester.pumpAndSettle();
    expect(find.text('Agents'), findsOneWidget);
    expect(find.text('Anyone'), findsNothing);
    expect(find.text('Deployed the fix'), findsOneWidget);
    expect(find.text('Hey check this out'), findsNothing);
    await _chooseActivityType(tester, 'Mentions');
    await tester.pumpAndSettle();
    expect(find.text('No mentions yet'), findsOneWidget);
    await _chooseActivityType(tester, 'Reminders');
    await tester.pumpAndSettle();
    expect(find.text('No reminders'), findsOneWidget);
    await _chooseActivityType(tester, 'Drafts');
    await tester.pumpAndSettle();
    expect(find.text('No drafts'), findsOneWidget);
  });

  testWidgets('iOS renders both Activity menus as glass controls', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(390, 844));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      await buildTestable(knownAgentPubkeys: const {'bob_pk'}),
    );
    await tester.pumpAndSettle();
    IosMessageCreateButton sender() => tester.widget<IosMessageCreateButton>(
      find.byKey(const ValueKey('activity-sender-menu')),
    );
    for (final key in ['activity-type-menu', 'activity-sender-menu']) {
      final view = tester.widget<UiKitView>(
        find.descendant(
          of: find.byKey(ValueKey(key)),
          matching: find.byType(UiKitView),
        ),
      );
      expect(view.creationParams, containsPair('plain', false));
    }
    final close = tester.widget<IosGlassNavigationButton>(
      find.byWidgetPredicate(
        (w) =>
            w is IosGlassNavigationButton &&
            w.icon == IosGlassNavigationIcon.close,
      ),
    );
    expect(close.semanticLabel, 'Close activity');
    expect(close.controlSize, 44);
    expect(close.onPressed, isNotNull);
    final type = tester.widget<IosMessageCreateButton>(
      find.byKey(const ValueKey('activity-type-menu')),
    );
    expect(type.label, 'All');
    expect(type.accessibilityLabel, 'Activity type, All');
    expect(type.actions.map((a) => a.label), [
      'All',
      'Threads',
      'Mentions',
      'DMs',
      'Reminders',
      'Drafts',
    ]);
    type.actions[1].onPressed!();
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<IosMessageCreateButton>(
            find.byKey(const ValueKey('activity-type-menu')),
          )
          .label,
      'Threads',
    );
    expect(
      tester
          .widget<IosMessageCreateButton>(
            find.byKey(const ValueKey('activity-type-menu')),
          )
          .accessibilityLabel,
      'Activity type, Threads',
    );
    type.actions.first.onPressed!();
    await tester.pumpAndSettle();
    expect(sender().label, 'Anyone');
    expect(find.byKey(const ValueKey('activity-options-menu')), findsNothing);
    expect(sender().icon, 'chevronDown');
    expect(sender().showIconWithLabel, isTrue);
    expect(sender().eagerGestures, isFalse);
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('activity-filters')),
        matching: find.byKey(const ValueKey('activity-sender-menu')),
      ),
      findsOneWidget,
    );
    sender().actions.last.onPressed!();
    await tester.pumpAndSettle();
    expect(sender().label, 'Agents');
    expect(sender().selectedIndex, 2);
    expect(find.text('Deployed the fix'), findsOneWidget);
    expect(find.text('Hey check this out'), findsNothing);
    sender().actions[1].onPressed!();
    await tester.pumpAndSettle();
    expect(sender().label, 'People');
    expect(find.text('Hey check this out'), findsOneWidget);
    tester
        .widget<IosGlassNavigationButton>(
          find.byKey(const ValueKey('activity-unread-filter')),
        )
        .onPressed!();
    await tester.pumpAndSettle();
    expect(find.text('Hey check this out'), findsOneWidget);
    expect(find.text('Deployed the fix'), findsNothing);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpAndSettle();
  }, variant: TargetPlatformVariant({TargetPlatform.iOS}));

  testWidgets('Activity filter controls remain usable with large phone text', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(390, 844));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      await buildTestable(textScaler: const TextScaler.linear(3)),
    );
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    await _chooseActivityType(tester, 'DMs');
    await tester.pumpAndSettle();
    expect(find.text('No direct messages yet'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}

Future<void> _chooseActivityType(WidgetTester tester, String label) async {
  final trigger = find.byKey(const ValueKey('activity-type-menu'));
  await tester.ensureVisible(trigger);
  await tester.pumpAndSettle();
  await tester.tap(trigger);
  await tester.pumpAndSettle();
  await tester.tap(
    find.descendant(
      of: find.byKey(const ValueKey('activity-type-popover')),
      matching: find.text(label),
    ),
  );
  await tester.pumpAndSettle();
}
