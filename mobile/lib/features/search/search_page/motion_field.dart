part of '../search_page.dart';

const _searchIdleIconSize = 26.0;
const _searchCompactIconSize = 18.0;
const _searchFieldHint = 'Search messages, channels, and people';
const _searchIdleIconInset = Grid.xxs;
const _searchIdleTextInset =
    _searchIdleIconInset + _searchIdleIconSize + Grid.xxs;
const _searchCompactTextInset =
    _searchIdleIconInset + _searchCompactIconSize + Grid.xxs;

class _SearchMotionField extends StatelessWidget {
  final TextEditingController controller;
  final FocusNode focusNode;
  final Color iconColor;
  final Color inputColor;
  final Color placeholderColor;
  final Color surfaceColor;
  final bool isSearchEditing;
  final bool reduceMotion;
  final Duration motionDuration;
  final VoidCallback onTap;
  final ValueChanged<String> onChanged;
  final ValueChanged<String> onSubmitted;

  const _SearchMotionField({
    required this.controller,
    required this.focusNode,
    required this.iconColor,
    required this.inputColor,
    required this.placeholderColor,
    required this.surfaceColor,
    required this.isSearchEditing,
    required this.reduceMotion,
    required this.motionDuration,
    required this.onTap,
    required this.onChanged,
    required this.onSubmitted,
  });

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
                key: const Key('search-field'),
                controller: controller,
                focusNode: focusNode,
                decoration: InputDecoration(
                  hintText: isSearchEditing ? null : _searchFieldHint,
                  hintStyle: searchInputTextStyle.copyWith(
                    color: placeholderColor,
                    fontSize: _searchIdleTextSize,
                    height: 20 / _searchIdleTextSize,
                  ),
                  border: InputBorder.none,
                  enabledBorder: InputBorder.none,
                  focusedBorder: InputBorder.none,
                  isDense: true,
                  contentPadding: EdgeInsets.only(
                    left: isSearchEditing
                        ? _searchCompactTextInset
                        : _searchIdleTextInset,
                    right: Grid.xxs,
                    top: isSearchEditing ? _searchFieldVerticalPadding : 0,
                    bottom: isSearchEditing ? _searchFieldVerticalPadding : 0,
                  ),
                ),
                style: searchInputTextStyle.copyWith(color: inputColor),
                textAlignVertical: TextAlignVertical.center,
                textAlign: TextAlign.start,
                textInputAction: TextInputAction.search,
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
                scale: isSearchEditing
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
