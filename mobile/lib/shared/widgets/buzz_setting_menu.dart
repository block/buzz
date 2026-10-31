import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';

import '../theme/buzz_icons.dart';
import '../theme/theme.dart';
import 'anchored_popover_menu.dart';
import 'app_list.dart';
import 'ios_message_create_button.dart';

/// One choice in a [BuzzSettingMenu].
class BuzzSettingMenuOption<T> {
  final Key? key;
  final String label;
  final T value;

  const BuzzSettingMenuOption({
    this.key,
    required this.label,
    required this.value,
  });
}

/// A bordered settings row showing its current value, opening a checked menu.
class BuzzSettingMenu<T> extends StatelessWidget {
  final Key controlKey;
  final bool enabled;
  final String label;
  final ValueChanged<T> onSelected;
  final List<BuzzSettingMenuOption<T>> options;
  final T? value;
  final IconData? valueIcon;
  final String valueLabel;

  /// Use the same borderless row as Settings inside a shared card.
  final bool grouped;

  /// Affordance for opening the menu.
  final IconData trailingIcon;

  const BuzzSettingMenu({
    super.key,
    required this.controlKey,
    required this.enabled,
    required this.label,
    required this.onSelected,
    required this.options,
    required this.value,
    this.valueIcon,
    required this.valueLabel,
    this.grouped = false,
    this.trailingIcon = BuzzIcons.chevronDown,
  });

  @override
  Widget build(BuildContext context) {
    if (grouped && defaultTargetPlatform == TargetPlatform.iOS) {
      return Stack(
        children: [
          ExcludeSemantics(
            child: Opacity(
              opacity: enabled ? 1 : 0.5,
              child: AppListRow(
                title: label,
                value: valueLabel,
                trailing: Icon(
                  trailingIcon,
                  size: 16,
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
          ),
          Positioned.fill(
            child: LayoutBuilder(
              builder: (context, constraints) => IosMessageCreateButton(
                key: controlKey,
                label: '',
                accessibilityLabel: '$label: $valueLabel',
                width: constraints.maxWidth,
                height: constraints.maxHeight,
                enabled: enabled,
                eagerGestures: false,
                selectedIndex: options.indexWhere(
                  (option) => option.value == value,
                ),
                actions: [
                  for (final option in options)
                    (
                      label: option.label,
                      symbol: '',
                      onPressed: enabled
                          ? () => onSelected(option.value)
                          : null,
                    ),
                ],
              ),
            ),
          ),
        ],
      );
    }

    void toggleMenu(MenuController menu) {
      if (menu.isOpen) return menu.close();
      FocusManager.instance.primaryFocus?.unfocus();
      menu.open();
    }

    final shape = RoundedRectangleBorder(
      side: BorderSide(color: context.colors.outlineVariant),
      borderRadius: BorderRadius.circular(Radii.container),
    );
    // The menu spans the row, like Material's dropdown menus.
    return LayoutBuilder(
      builder: (context, constraints) => MenuAnchor(
        alignmentOffset: const Offset(0, Grid.half),
        // Match the app's shared popover surface.
        style: MenuStyle(
          shape: WidgetStatePropertyAll(appPopoverShape(context)),
          backgroundColor: WidgetStatePropertyAll(appPopoverColor(context)),
          shadowColor: WidgetStatePropertyAll(appPopoverShadowColor(context)),
          elevation: const WidgetStatePropertyAll(appPopoverElevation),
          padding: const WidgetStatePropertyAll(
            EdgeInsets.symmetric(vertical: Grid.half),
          ),
        ),
        menuChildren: [
          for (final option in options)
            // The check icon carries no semantics, so mark the selection on
            // the item's own node.
            MergeSemantics(
              child: Semantics(
                checked: option.value == value,
                inMutuallyExclusiveGroup: true,
                child: MenuItemButton(
                  key: option.key,
                  onPressed: () => onSelected(option.value),
                  style: MenuItemButton.styleFrom(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Grid.twelve,
                    ),
                  ),
                  // A trailing check keeps labels flush with the menu's edge.
                  trailingIcon: option.value == value
                      ? Icon(
                          Icons.check,
                          size: 20,
                          color: context.colors.primary,
                        )
                      : const SizedBox(width: 20),
                  // Root menu panels ignore size styles and shrink-wrap their items,
                  // so the label sets the width: the row's, less the item's padding,
                  // the check and the gap before it.
                  child: SizedBox(
                    width: constraints.maxWidth - Grid.twelve * 3 - 20,
                    child: Text(option.label),
                  ),
                ),
              ),
            ),
        ],
        builder: (context, menu, _) => Semantics(
          button: true,
          enabled: enabled,
          expanded: menu.isOpen,
          child: Tooltip(
            message: '$label: $valueLabel',
            // Draw on an ink surface so the tap ripple shows inside the border.
            child: grouped
                ? Opacity(
                    opacity: enabled ? 1 : 0.5,
                    child: AppListRow(
                      key: controlKey,
                      title: label,
                      value: valueLabel,
                      trailing: Icon(
                        trailingIcon,
                        size: 16,
                        color: context.colors.onSurfaceVariant,
                      ),
                      onTap: enabled ? () => toggleMenu(menu) : null,
                    ),
                  )
                : Material(
                    color: context.colors.surface,
                    shape: shape,
                    clipBehavior: Clip.antiAlias,
                    child: InkWell(
                      onTap: enabled ? () => toggleMenu(menu) : null,
                      child: Padding(
                        key: controlKey,
                        padding: const EdgeInsets.symmetric(
                          horizontal: Grid.twelve,
                          vertical: Grid.xs,
                        ),
                        child: Opacity(
                          opacity: enabled ? 1 : 0.5,
                          child: Row(
                            children: [
                              Expanded(
                                child: Text(
                                  label,
                                  style: context.textTheme.labelLarge?.copyWith(
                                    color: context.colors.onSurface,
                                    fontWeight: FontWeight.w600,
                                  ),
                                ),
                              ),
                              if (valueIcon case final icon?) ...[
                                Icon(
                                  icon,
                                  size: 16,
                                  color: context.colors.onSurfaceVariant,
                                ),
                                const SizedBox(width: Grid.half),
                              ],
                              // Long values truncate instead of squeezing the label.
                              ConstrainedBox(
                                constraints: const BoxConstraints(
                                  maxWidth: 180,
                                ),
                                child: Text(
                                  valueLabel,
                                  overflow: TextOverflow.ellipsis,
                                  style: context.textTheme.bodyMedium?.copyWith(
                                    color: context.colors.onSurface,
                                    fontWeight: FontWeight.w500,
                                  ),
                                ),
                              ),
                              const SizedBox(width: Grid.half),
                              Icon(
                                trailingIcon,
                                size: 16,
                                color: context.colors.onSurfaceVariant,
                              ),
                            ],
                          ),
                        ),
                      ),
                    ),
                  ),
          ),
        ),
      ),
    );
  }
}
