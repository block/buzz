import 'dart:ui' show CheckedState, Tristate;

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_setting_menu.dart';

void main() {
  Widget menu({required bool enabled}) => MaterialApp(
    theme: AppTheme.light(),
    home: Scaffold(
      body: BuzzSettingMenu<String>(
        controlKey: const Key('menu'),
        enabled: enabled,
        label: 'Effort',
        onSelected: (_) {},
        options: const [
          BuzzSettingMenuOption(key: Key('low'), label: 'Low', value: 'low'),
          BuzzSettingMenuOption(key: Key('high'), label: 'High', value: 'high'),
        ],
        value: 'high',
        valueLabel: 'High',
      ),
    ),
  );

  testWidgets('form menu preserves keyboard focus through selection', (
    tester,
  ) async {
    final focus = FocusNode();
    addTearDown(focus.dispose);
    String? selected;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Column(
            children: [
              TextField(focusNode: focus),
              BuzzSettingMenu<String>(
                controlKey: const Key('form-menu'),
                enabled: true,
                grouped: true,
                preserveTextInput: true,
                label: 'Visibility',
                onSelected: (value) => selected = value,
                options: const [
                  BuzzSettingMenuOption(
                    key: Key('private'),
                    label: 'Private',
                    value: 'private',
                  ),
                ],
                value: 'open',
                valueLabel: 'Public',
              ),
            ],
          ),
        ),
      ),
    );
    await tester.tap(find.byType(TextField));
    await tester.pumpAndSettle();
    expect(focus.hasFocus, isTrue);
    expect(tester.testTextInput.isVisible, isTrue);
    await tester.tap(find.byKey(const Key('form-menu')));
    await tester.pumpAndSettle();
    expect(focus.hasFocus, isTrue);
    expect(tester.testTextInput.isVisible, isTrue);
    await tester.tap(find.byKey(const Key('private')));
    await tester.pumpAndSettle();
    expect(selected, 'private');
    expect(focus.hasFocus, isTrue);
    expect(tester.testTextInput.isVisible, isTrue);
  });

  testWidgets('exposes button, expanded and checked semantics', (tester) async {
    final handle = tester.ensureSemantics();
    await tester.pumpWidget(menu(enabled: true));
    SemanticsData trigger() =>
        tester.getSemantics(find.byKey(const Key('menu'))).getSemanticsData();
    expect(trigger().flagsCollection.isButton, isTrue);
    expect(trigger().flagsCollection.isEnabled, Tristate.isTrue);
    expect(trigger().flagsCollection.isExpanded, Tristate.isFalse);

    await tester.tap(find.byKey(const Key('menu')));
    await tester.pumpAndSettle();
    expect(trigger().flagsCollection.isExpanded, Tristate.isTrue);
    SemanticsData option(String key) =>
        tester.getSemantics(find.byKey(Key(key))).getSemanticsData();
    expect(option('high').flagsCollection.isChecked, CheckedState.isTrue);
    final check = find.descendant(
      of: find.byKey(const Key('high')),
      matching: find.byIcon(Icons.check),
    );
    final label = find.descendant(
      of: find.byKey(const Key('high')),
      matching: find.text('High'),
    );
    expect(tester.getCenter(check).dx, lessThan(tester.getTopLeft(label).dx));

    expect(option('low').flagsCollection.isChecked, CheckedState.isFalse);
    handle.dispose();
  });

  testWidgets('a disabled menu reads as a disabled button', (tester) async {
    final handle = tester.ensureSemantics();
    await tester.pumpWidget(menu(enabled: false));
    final data = tester
        .getSemantics(find.byKey(const Key('menu')))
        .getSemanticsData();
    expect(data.flagsCollection.isButton, isTrue);
    expect(data.flagsCollection.isEnabled, Tristate.isFalse);
    handle.dispose();
  });
  testWidgets('iOS grouped choices use the native menu and update selection', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/navigation_glass/987');
    final calls = <MethodCall>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      calls.add(call);
      return null;
    });
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      ),
    );
    String? selected;
    Widget nativeMenu({bool enabled = true, String value = 'high'}) =>
        MaterialApp(
          home: Scaffold(
            body: BuzzSettingMenu<String>(
              controlKey: const Key('native-menu'),
              grouped: true,
              enabled: enabled,
              label: 'Thinking',
              onSelected: (value) => selected = value,
              options: const [
                BuzzSettingMenuOption(label: 'Low', value: 'low'),
                BuzzSettingMenuOption(label: 'High', value: 'high'),
              ],
              value: value,
              valueLabel: value,
            ),
          ),
        );
    await tester.binding.setSurfaceSize(const Size(340, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(nativeMenu(enabled: false));
    final view = tester.widget<UiKitView>(find.byType(UiKitView));
    expect(view.viewType, 'buzz/navigation_glass');
    expect(view.creationParams, containsPair('deferMenuActions', false));
    expect(view.creationParams, containsPair('fillWidth', true));
    expect(view.gestureRecognizers, isEmpty);
    expect(find.byType(MenuAnchor), findsNothing);
    expect(
      view.creationParams,
      containsPair('accessibilityLabel', 'Thinking: high'),
    );
    expect(
      ((view.creationParams as Map)['menu'] as List)[1],
      containsPair('checked', true),
    );
    await tester.pumpAndSettle();
    view.onPlatformViewCreated!(987);
    await tester.pump();
    Future<void> choose(int index) async {
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        channel.name,
        const StandardMethodCodec().encodeMethodCall(
          MethodCall('menuAction', index),
        ),
        (_) {},
      );
      await tester.pump();
    }

    await tester.pumpWidget(nativeMenu());
    await tester.pump();
    final portraitWidth = tester.getSize(find.byType(UiKitView)).width;
    await tester.binding.setSurfaceSize(const Size(600, 340));
    await tester.pump();
    expect(
      tester.getSize(find.byType(UiKitView)).width,
      greaterThan(portraitWidth),
    );
    await choose(0);
    expect(selected, 'low');
    await tester.pumpWidget(nativeMenu(value: 'low'));
    await tester.pump();
    final content =
        calls.lastWhere((call) => call.method == 'setContent').arguments as Map;
    expect((content['menu'] as List)[0], containsPair('checked', true));
    await tester.pumpWidget(nativeMenu(enabled: false));
    await tester.pump();
    expect(
      calls.lastWhere((call) => call.method == 'setAppearance').arguments,
      containsPair('enabled', false),
    );
    await choose(1);
    expect(selected, 'low');
    await tester.pumpWidget(nativeMenu(value: 'low'));
    await tester.pump();
    await choose(1);
    expect(selected, 'high');
    await tester.pumpWidget(const SizedBox.shrink());
    debugDefaultTargetPlatformOverride = null;
  });
}
