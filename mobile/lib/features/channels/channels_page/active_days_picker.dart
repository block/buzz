import 'package:flutter/cupertino.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/buzz_sheet_header.dart';
import '../../../shared/widgets/ios_glass_navigation_button.dart';
import '../../../shared/widgets/modal_presentation.dart';
import '../channel_filter/channel_filter_store.dart';

Future<int?> showActiveDaysPicker(BuildContext context, int initial) async {
  ModalRoute<dynamic>? route;
  final days = await showBuzzModalBottomSheet<int>(
    context: context,
    nativeIosBarrier: defaultTargetPlatform == TargetPlatform.iOS,
    showCloseButton: false,
    showDragHandle: false,
    enableDrag: false,
    isScrollControlled: true,
    builder: (context) {
      route = ModalRoute.of(context);
      return _ActiveDaysPicker(initial: initial);
    },
  );
  await route?.completed;
  return days;
}

class _ActiveDaysPicker extends HookWidget {
  const _ActiveDaysPicker({required this.initial});
  final int initial;

  @override
  Widget build(BuildContext context) {
    final ios = defaultTargetPlatform == TargetPlatform.iOS;
    final native = useState<MethodChannel?>(null);
    final selected = useRef(initial);
    final controller = useFixedExtentScrollController(initialItem: initial - 1);
    Future<void> confirm() async {
      final route = ModalRoute.of(context);
      final days = ios
          ? await native.value?.invokeMethod<int>('selection')
          : selected.value;
      if (days != null && context.mounted && route?.isCurrent == true) {
        Navigator.pop(context, days);
      }
    }

    void cancel() => Navigator.pop(context);
    return SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          BuzzSheetHeader(
            title: 'Custom period',
            leading: ios
                ? IosGlassNavigationButton(
                    icon: IosGlassNavigationIcon.close,
                    semanticLabel: 'Cancel',
                    onPressed: cancel,
                  )
                : IconButton(
                    tooltip: 'Cancel',
                    icon: const Icon(BuzzIcons.x),
                    onPressed: cancel,
                  ),
            trailing: ios
                ? IosGlassNavigationButton(
                    icon: IosGlassNavigationIcon.check,
                    semanticLabel: 'Use period',
                    onPressed: native.value == null ? null : confirm,
                  )
                : IconButton(
                    tooltip: 'Use period',
                    icon: const Icon(BuzzIcons.check),
                    onPressed: confirm,
                  ),
          ),
          Flexible(
            child: SizedBox(
              height: 216,
              child: ios
                  ? UiKitView(
                      viewType: 'buzz/active_days_wheel',
                      creationParamsCodec: const StandardMessageCodec(),
                      creationParams: {
                        'initial': initial,
                        'maximum': maxActiveWindowDays,
                        'dark': Theme.of(context).brightness == Brightness.dark,
                      },
                      gestureRecognizers: {
                        Factory<EagerGestureRecognizer>(
                          EagerGestureRecognizer.new,
                        ),
                      },
                      onPlatformViewCreated: (id) {
                        if (context.mounted) {
                          native.value = MethodChannel(
                            'buzz/active_days_wheel/$id',
                          );
                        }
                      },
                    )
                  : CupertinoPicker.builder(
                      scrollController: controller,
                      itemExtent: 44,
                      childCount: maxActiveWindowDays,
                      onSelectedItemChanged: (row) => selected.value = row + 1,
                      itemBuilder: (context, row) => Center(
                        child: Text(row == 0 ? '1 day' : '${row + 1} days'),
                      ),
                    ),
            ),
          ),
        ],
      ),
    );
  }
}
