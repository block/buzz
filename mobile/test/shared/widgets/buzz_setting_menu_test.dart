import 'dart:ui' show CheckedState, Tristate;

import 'package:flutter/material.dart';
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
}
