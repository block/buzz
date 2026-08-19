import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'profile_provider.dart';

Future<void> showEditProfileSheet(
  BuildContext context, {
  required String initialName,
}) {
  return showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => _EditProfileSheet(initialName: initialName),
  );
}

class _EditProfileSheet extends ConsumerStatefulWidget {
  const _EditProfileSheet({required this.initialName});

  final String initialName;

  @override
  ConsumerState<_EditProfileSheet> createState() => _EditProfileSheetState();
}

class _EditProfileSheetState extends ConsumerState<_EditProfileSheet> {
  late final TextEditingController _controller;
  bool _isSaving = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _controller = TextEditingController(text: widget.initialName);
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    final name = _controller.text.trim();
    if (name.isEmpty || _isSaving) return;
    setState(() {
      _isSaving = true;
      _error = null;
    });
    try {
      await ref.read(profileProvider.notifier).saveDisplayName(name);
      if (mounted) Navigator.of(context).pop();
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _isSaving = false;
        _error = 'Buzz could not save your name. Try again.';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          Grid.sm,
          0,
          Grid.sm,
          MediaQuery.viewInsetsOf(context).bottom + Grid.sm,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('Edit profile', style: context.textTheme.titleLarge),
            const SizedBox(height: Grid.sm),
            TextField(
              key: const ValueKey('edit-profile-name'),
              controller: _controller,
              enabled: !_isSaving,
              autofocus: true,
              textCapitalization: TextCapitalization.words,
              autofillHints: const [AutofillHints.name],
              decoration: const InputDecoration(labelText: 'Your name'),
              onChanged: (_) => setState(() {}),
              onSubmitted: (_) => _save(),
            ),
            if (_error != null) ...[
              const SizedBox(height: Grid.xs),
              Text(
                _error!,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ],
            const SizedBox(height: Grid.sm),
            FilledButton(
              onPressed: _isSaving || _controller.text.trim().isEmpty
                  ? null
                  : _save,
              child: Text(_isSaving ? 'Saving…' : 'Save'),
            ),
          ],
        ),
      ),
    );
  }
}
