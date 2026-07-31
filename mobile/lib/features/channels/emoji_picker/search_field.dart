part of '../emoji_picker.dart';

/// Search field at the top of the picker sheet.
///
/// Desktop reaches into emoji-mart's shadow DOM to turn off spellcheck,
/// autocorrect, and autocapitalize on its search input (see
/// `disableSearchInputCorrections` in `EmojiPicker.tsx`) — shortcodes are not
/// words and the OS mangles them. Flutter exposes the same switches directly.
class _EmojiSearchField extends StatelessWidget {
  final TextEditingController controller;

  const _EmojiSearchField({required this.controller});

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Padding(
      padding: const EdgeInsets.fromLTRB(Grid.gutter, 0, Grid.gutter, Grid.xxs),
      child: TextField(
        key: const ValueKey('emoji-picker-search'),
        controller: controller,
        autocorrect: false,
        enableSuggestions: false,
        textCapitalization: TextCapitalization.none,
        textInputAction: TextInputAction.search,
        style: searchInputTextStyle.copyWith(color: colors.onSurface),
        decoration: InputDecoration(
          hintText: 'Search emoji',
          hintStyle: searchInputTextStyle.copyWith(
            color: colors.onSurfaceVariant,
          ),
          prefixIcon: Icon(
            LucideIcons.search,
            size: 18,
            color: colors.onSurfaceVariant,
          ),
          prefixIconConstraints: const BoxConstraints(
            minWidth: Grid.md,
            minHeight: Grid.md,
          ),
          suffixIcon: ValueListenableBuilder<TextEditingValue>(
            valueListenable: controller,
            builder: (context, value, _) {
              if (value.text.isEmpty) return const SizedBox.shrink();
              return IconButton(
                key: const ValueKey('emoji-picker-search-clear'),
                onPressed: controller.clear,
                icon: Icon(
                  LucideIcons.x,
                  size: 16,
                  color: colors.onSurfaceVariant,
                ),
                visualDensity: VisualDensity.compact,
                tooltip: 'Clear search',
              );
            },
          ),
          filled: true,
          fillColor: colors.surface,
          isDense: true,
          contentPadding: const EdgeInsets.symmetric(vertical: Grid.xxs),
          border: OutlineInputBorder(
            borderRadius: BorderRadius.circular(Radii.lg),
            borderSide: BorderSide(color: colors.outlineVariant),
          ),
          enabledBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(Radii.lg),
            borderSide: BorderSide(color: colors.outlineVariant),
          ),
          focusedBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(Radii.lg),
            borderSide: BorderSide(color: colors.primary),
          ),
        ),
      ),
    );
  }
}
