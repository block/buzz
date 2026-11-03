import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/me/me_assistants.dart';
import '../../../shared/theme/theme.dart';
import 'me_assistant_list.dart';

/// Opens immediately with cached assistants; first-load progress stays in-sheet.
class MeAssistantPickerSheet extends ConsumerWidget {
  const MeAssistantPickerSheet({super.key, required this.selected});
  final String? selected;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final assistants = ref.watch(meAssistantsProvider);
    if (assistants.value case final choices?) {
      return MeAssistantList(choices: choices, selected: selected);
    }
    return SizedBox(
      height: 240,
      child: Center(
        child: assistants.hasError
            ? TextButton(
                onPressed: () => ref.invalidate(meAssistantsProvider),
                child: const Text('Retry loading agents'),
              )
            : Padding(
                padding: const EdgeInsets.all(Grid.gutter),
                child: const CircularProgressIndicator(
                  semanticsLabel: 'Loading agents',
                ),
              ),
      ),
    );
  }
}
