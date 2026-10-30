import 'package:buzz/features/channels/message_actions/native_message_action_selection.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_test/flutter_test.dart';

Future<void> _select(WidgetTester tester, int viewId, String action) async {
  tester.binding.channelBuffers.push(
    'buzz/native_message_action_surface/$viewId',
    const StandardMethodCodec().encodeMethodCall(
      MethodCall('selected', {'id': action}),
    ),
    (_) {},
  );
  await tester.pump();
}

Widget _menu(int? viewId, ValueChanged<String> onSelected) => HookBuilder(
  builder: (_) {
    useNativeMessageActionSelection(viewId, onSelected);
    return const SizedBox();
  },
);

void main() {
  testWidgets('keeps native selections connected through callback rebuilds', (
    tester,
  ) async {
    final selections = <String>[];
    await tester.pumpWidget(_menu(null, (id) => selections.add('initial:$id')));
    await tester.pumpWidget(_menu(41, (id) => selections.add('created:$id')));
    await _select(tester, 41, 'copyText');
    expect(selections, ['created:copyText']);

    // Keyboard inset changes rebuild the popover with a fresh callback.
    for (var frame = 0; frame < 5; frame++) {
      await tester.pumpWidget(
        _menu(41, (id) => selections.add('frame$frame:$id')),
      );
    }
    await _select(tester, 41, 'edit');
    expect(selections, ['created:copyText', 'frame4:edit']);
  });

  testWidgets('moves the handler to a new view and removes it on dismissal', (
    tester,
  ) async {
    final selections = <String>[];
    await tester.pumpWidget(_menu(51, selections.add));
    await _select(tester, 51, 'copyText');
    await tester.pumpWidget(_menu(52, selections.add));
    await _select(tester, 51, 'edit');
    await _select(tester, 52, 'delete');
    expect(selections, ['copyText', 'delete']);

    await tester.pumpWidget(const SizedBox());
    await _select(tester, 52, 'edit');
    expect(selections, ['copyText', 'delete']);
  });
}
