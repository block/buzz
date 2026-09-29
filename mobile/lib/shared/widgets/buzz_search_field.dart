import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../theme/theme.dart';

/// Height of an idle [BuzzSearchField].
const double buzzSearchIdleFieldHeight = 45;

/// Font size used for idle [BuzzSearchField] text.
const double buzzSearchIdleTextSize = 15;
const double _searchIdleIconSize = 26;
const double _searchCompactIconSize = 18;
const double _searchIdleIconInset = Grid.xxs;
const double _searchIdleTextInset =
    _searchIdleIconInset + _searchIdleIconSize + Grid.xxs;
const double _searchCompactTextInset =
    _searchIdleIconInset + _searchCompactIconSize + Grid.xxs;

/// Buzz's global-search text field treatment, shared by search-like inputs.
class BuzzSearchField extends StatelessWidget {
  /// Creates a search field with Buzz's shared styling.
  const BuzzSearchField({
    required this.controller,
    required this.focusNode,
    required this.hintText,
    required this.iconColor,
    required this.inputColor,
    required this.placeholderColor,
    required this.surfaceColor,
    required this.isEditing,
    required this.reduceMotion,
    required this.motionDuration,
    required this.onTap,
    required this.onChanged,
    required this.onSubmitted,
    this.fieldKey = const Key('search-field'),
    this.autocorrect = true,
    this.enableSuggestions = true,
    this.enabled = true,
    this.textInputAction = TextInputAction.search,
    super.key,
  });

  /// Controller that owns the field's editable text.
  final TextEditingController controller;

  /// Node that controls the field's focus.
  final FocusNode focusNode;

  /// Placeholder shown while the field is idle and empty.
  final String hintText;

  /// Color applied to the search icon.
  final Color iconColor;

  /// Color applied to entered text.
  final Color inputColor;

  /// Color applied to [hintText].
  final Color placeholderColor;

  /// Background color of the field.
  final Color surfaceColor;

  /// Whether the field is in its compact editing state.
  final bool isEditing;

  /// Whether to suppress field animations for reduced-motion users.
  final bool reduceMotion;

  /// Duration used by the field's editing-state animation.
  final Duration motionDuration;

  /// Called when the field is tapped.
  final VoidCallback onTap;

  /// Called whenever the field's text changes.
  final ValueChanged<String> onChanged;

  /// Called when the user submits the field.
  final ValueChanged<String> onSubmitted;

  /// Key assigned to the underlying text field.
  final Key fieldKey;

  /// Whether the text field should autocorrect user input.
  final bool autocorrect;

  /// Whether the platform should offer text suggestions.
  final bool enableSuggestions;

  /// Whether the text field accepts user input.
  final bool enabled;

  /// Action displayed on the keyboard's submit key.
  final TextInputAction textInputAction;

  @override
  Widget build(BuildContext context) => DecoratedBox(
    decoration: BoxDecoration(
      color: surfaceColor,
      borderRadius: BorderRadius.circular(Radii.lg),
    ),
    child: Stack(
      children: [
        Positioned.fill(
          child: Align(
            alignment: Alignment.centerLeft,
            child: SizedBox(
              width: double.infinity,
              child: TextField(
                key: fieldKey,
                controller: controller,
                focusNode: focusNode,
                autocorrect: autocorrect,
                enableSuggestions: enableSuggestions,
                enabled: enabled,
                decoration: InputDecoration(
                  hintText: isEditing ? null : hintText,
                  hintStyle: searchInputTextStyle.copyWith(
                    color: placeholderColor,
                    fontSize: buzzSearchIdleTextSize,
                    height: 20 / buzzSearchIdleTextSize,
                  ),
                  border: InputBorder.none,
                  enabledBorder: InputBorder.none,
                  focusedBorder: InputBorder.none,
                  isDense: true,
                  contentPadding: EdgeInsets.only(
                    left: isEditing
                        ? _searchCompactTextInset
                        : _searchIdleTextInset,
                    right: Grid.xxs,
                    top: isEditing ? Grid.xxs : 0,
                    bottom: isEditing ? Grid.xxs : 0,
                  ),
                ),
                style: searchInputTextStyle.copyWith(color: inputColor),
                textAlignVertical: TextAlignVertical.center,
                textAlign: TextAlign.start,
                textInputAction: textInputAction,
                onTap: onTap,
                onChanged: onChanged,
                onSubmitted: onSubmitted,
              ),
            ),
          ),
        ),
        IgnorePointer(
          child: Align(
            alignment: Alignment.centerLeft,
            child: Padding(
              padding: const EdgeInsets.only(left: _searchIdleIconInset),
              child: AnimatedScale(
                duration: reduceMotion ? Duration.zero : motionDuration,
                curve: Curves.easeInOutCubic,
                scale: isEditing
                    ? _searchCompactIconSize / _searchIdleIconSize
                    : 1,
                child: Icon(
                  LucideIcons.search,
                  key: const Key('search-moving-icon'),
                  size: _searchIdleIconSize,
                  color: iconColor,
                ),
              ),
            ),
          ),
        ),
      ],
    ),
  );
}
