import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../shared/theme/grid.dart';

/// Reuses the account flow's scoped profile writer; skipping publishes nothing.
class HostedProfileSetup extends HookConsumerWidget {
  const HostedProfileSetup({super.key, required this.save, required this.skip});
  final Future<void> Function(String) save;
  final VoidCallback skip;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final name = useTextEditingController();
    useListenable(name);
    final busy = useState(false);
    final error = useState(false);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TextField(
          controller: name,
          maxLength: 100,
          enabled: !busy.value,
          textCapitalization: TextCapitalization.words,
          decoration: const InputDecoration(labelText: 'Display name'),
        ),
        if (error.value)
          const Text('We could not save your name. Try again or skip for now.'),
        const SizedBox(height: Grid.sm),
        FilledButton(
          onPressed: busy.value || name.text.trim().isEmpty
              ? null
              : () async {
                  if (name.text.trim().isEmpty) return;
                  busy.value = true;
                  error.value = false;
                  try {
                    await save(name.text.trim());
                  } catch (_) {
                    if (context.mounted) error.value = true;
                  } finally {
                    if (context.mounted) busy.value = false;
                  }
                },
          child: Text(busy.value ? 'Saving…' : 'Continue'),
        ),
        TextButton(
          onPressed: busy.value ? null : skip,
          child: const Text('Skip for now'),
        ),
      ],
    );
  }
}
