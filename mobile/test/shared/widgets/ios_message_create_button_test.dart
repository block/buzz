import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/ios_message_create_button.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('trailing icon slot preserves the full native hit target', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const Scaffold(
            body: IosMessageCreateButton(
              icon: 'more',
              width: 44,
              height: 44,
              trailingIconSlotWidth: 20,
              actions: [],
            ),
          ),
        ),
      ),
    );
    final view = tester.widget<UiKitView>(find.byType(UiKitView));
    final params = view.creationParams as Map;
    expect(tester.getSize(find.byType(UiKitView)), const Size(44, 44));
    expect(params['buttonCenterX'], 34);
    expect(params['controlWidth'], 20);
    expect(params['hitTargetWidth'], 44);
    expect(params['hitTargetHeight'], 44);
  });
}
