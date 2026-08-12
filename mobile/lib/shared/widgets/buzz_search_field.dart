import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../theme/theme.dart';

const double buzzSearchIdleFieldHeight = 45;
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

  final TextEditingController controller;
  final FocusNode focusNode;
  final String hintText;
  final Color iconColor;
  final Color inputColor;
  final Color placeholderColor;
  final Color surfaceColor;
  final bool isEditing;
  final bool reduceMotion;
  final Duration motionDuration;
  final VoidCallback onTap;
  final ValueChanged<String> onChanged;
  final ValueChanged<String> onSubmitted;
  final Key fieldKey;
  final bool autocorrect;
  final bool enableSuggestions;
  final bool enabled;
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
