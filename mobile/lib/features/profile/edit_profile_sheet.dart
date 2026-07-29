import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import 'profile_provider.dart';
import 'user_profile.dart';

const _saveButtonHeight = 52.0;

void showEditProfileSheet(
  BuildContext context, {
  required UserProfile? currentProfile,
}) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => _EditProfileSheet(currentProfile: currentProfile),
  );
}

class _EditProfileSheet extends HookConsumerWidget {
  const _EditProfileSheet({required this.currentProfile});

  final UserProfile? currentProfile;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final initialName = currentProfile?.displayName ?? '';
    final controller = useTextEditingController(text: initialName);
    final displayName = useState(initialName);
    final isSaving = useState(false);
    final errorMessage = useState<String?>(null);

    useEffect(() {
      void listener() {
        displayName.value = controller.text;
        errorMessage.value = null;
      }

      controller.addListener(listener);
      return () => controller.removeListener(listener);
    }, [controller]);

    final trimmedName = displayName.value.trim();
    final canSave =
        trimmedName.isNotEmpty &&
        trimmedName != initialName.trim() &&
        !isSaving.value;

    Future<void> handleSave() async {
      if (!canSave) return;
      isSaving.value = true;
      errorMessage.value = null;
      try {
        await ref.read(profileProvider.notifier).updateDisplayName(trimmedName);
        if (context.mounted) Navigator.of(context).pop();
      } catch (_) {
        if (!context.mounted) return;
        errorMessage.value = 'Could not update your profile. Try again.';
      } finally {
        if (context.mounted) isSaving.value = false;
      }
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        0,
        Grid.gutter,
        Grid.gutter +
            math.max(
              MediaQuery.viewInsetsOf(context).bottom,
              MediaQuery.viewPaddingOf(context).bottom,
            ),
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Edit profile', style: context.textTheme.titleMedium),
          const SizedBox(height: Grid.half),
          Text(
            'Choose the name people see across Buzz.',
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: Grid.twelve),
          TextField(
            controller: controller,
            autofocus: true,
            inputFormatters: const [
              _RuneLengthLimitingTextInputFormatter(
                maxProfileDisplayNameLength,
              ),
            ],
            textCapitalization: TextCapitalization.words,
            textInputAction: TextInputAction.done,
            decoration: InputDecoration(
              labelText: 'Display name',
              counterText:
                  '${displayName.value.runes.length}/'
                  '$maxProfileDisplayNameLength',
            ),
            onSubmitted: (_) => handleSave(),
          ),
          if (errorMessage.value case final message?)
            Padding(
              padding: const EdgeInsets.only(top: Grid.half),
              child: Text(
                message,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ),
          const SizedBox(height: Grid.gutter),
          SizedBox(
            width: double.infinity,
            height: _saveButtonHeight,
            child: FilledButton(
              onPressed: canSave ? handleSave : null,
              child: const Text('Save'),
            ),
          ),
        ],
      ),
    );
  }
}

class _RuneLengthLimitingTextInputFormatter extends TextInputFormatter {
  const _RuneLengthLimitingTextInputFormatter(this.maxLength);

  final int maxLength;

  @override
  TextEditingValue formatEditUpdate(
    TextEditingValue oldValue,
    TextEditingValue newValue,
  ) {
    if (newValue.text.runes.length <= maxLength) return newValue;

    final text = String.fromCharCodes(newValue.text.runes.take(maxLength));
    return TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
  }
}
