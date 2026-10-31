import 'package:flutter/cupertino.dart';

import '../theme/theme.dart';

/// Borderless community address field used by both landing and Add entry.
class IosCommunityLinkInput extends StatelessWidget {
  const IosCommunityLinkInput({
    super.key,
    required this.controller,
    this.focusNode,
    this.autofocus = false,
    this.enabled = true,
    this.onChanged,
    this.onSubmitted,
  });

  final TextEditingController controller;
  final FocusNode? focusNode;
  final bool autofocus;
  final bool enabled;
  final ValueChanged<String>? onChanged;
  final ValueChanged<String>? onSubmitted;

  @override
  Widget build(BuildContext context) => CupertinoTextField(
    key: const Key('community-link-input'),
    controller: controller,
    focusNode: focusNode,
    autofocus: autofocus,
    enabled: enabled,
    placeholder: 'Community link or address',
    placeholderStyle: context.textTheme.titleLarge?.copyWith(
      color: CupertinoColors.placeholderText.resolveFrom(context),
    ),
    keyboardType: TextInputType.url,
    textInputAction: TextInputAction.go,
    autocorrect: false,
    enableSuggestions: false,
    padding: const EdgeInsets.symmetric(vertical: Grid.xs),
    style: context.textTheme.titleLarge?.copyWith(
      color: context.colors.onSurface,
    ),
    cursorColor: context.colors.primary,
    decoration: null,
    onChanged: onChanged,
    onSubmitted: onSubmitted,
  );
}
