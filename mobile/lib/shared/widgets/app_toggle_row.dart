import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'app_list.dart';
import 'buzz_switch.dart';

/// A settings row with one accessible switch and a full-row touch target.
class AppToggleRow extends StatelessWidget {
  const AppToggleRow({
    super.key,
    this.rowKey,
    required this.title,
    this.subtitle,
    required this.value,
    required this.onChanged,
  });
  final Key? rowKey;
  final String title;
  final String? subtitle;
  final bool value;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    final native = defaultTargetPlatform == TargetPlatform.iOS;
    final content = Stack(
      alignment: Alignment.centerRight,
      children: [
        ExcludeSemantics(
          excluding: native,
          child: AppListRow(
            key: rowKey,
            title: title,
            subtitle: subtitle,
            trailing: const SizedBox(width: 60),
            onTap: onChanged == null ? null : () => onChanged!(!value),
          ),
        ),
        Positioned.fill(
          right: Grid.xs,
          child: Align(
            alignment: Alignment.centerRight,
            child: BuzzSwitch(
              value: value,
              label: subtitle == null ? title : '$title. $subtitle',
              onChanged: onChanged,
            ),
          ),
        ),
      ],
    );
    return native ? content : MergeSemantics(child: content);
  }
}
