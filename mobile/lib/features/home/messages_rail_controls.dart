import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/theme/buzz_icons.dart';
import '../../shared/widgets/agent_navigation_overlay.dart';
import '../../shared/widgets/agent_rail_link.dart';
import '../../shared/widgets/duo_toolbar.dart';
import '../../shared/widgets/ios_navigation_bar.dart';
import '../../shared/widgets/navigation_button_surface.dart';

/// Home's Search, New and Bestie as native rail controls, in one glass capsule
/// at the bottom of the Duo rail.
///
/// [agent] is the Bestie shortcut. It stays mounted at the capsule's bottom
/// slot, unseen while idle, to supply the native Bestie control and to draw
/// the voice pill while that control is held.
class MessagesRailControls extends HookWidget {
  const MessagesRailControls({
    super.key,
    required this.strip,
    required this.onSearch,
    required this.creationActions,
    required this.agent,
    this.onRecordingChanged,
  });

  final FoldStrip strip;
  final VoidCallback onSearch;
  final List<({String label, String symbol, VoidCallback? onPressed})>
  creationActions;
  final Widget agent;
  final ValueChanged<bool>? onRecordingChanged;

  @override
  Widget build(BuildContext context) {
    final link = useMemoized(AgentRailLink.new);
    final bestie = useValueListenable(link.action);
    final overlay = useValueNotifier(false);
    final recording = useState(false);
    // Reserve the Bestie slot before the shortcut publishes, so the capsule
    // never changes height.
    final actions = [
      IosNavigationAction(
        label: 'Search',
        symbol: 'magnifyingglass',
        icon: BuzzIcons.search300,
        onPressed: onSearch,
      ),
      IosNavigationAction(
        label: 'New',
        symbol: 'plus',
        icon: BuzzIcons.plus,
        children: [
          for (final action in creationActions)
            IosNavigationAction(
              label: action.label,
              symbol: action.symbol,
              onPressed: action.onPressed,
            ),
        ],
      ),
      bestie ?? const IosNavigationAction(label: 'Bestie', symbol: 'agent'),
    ];
    assert(actions.length == duoRailBottomControls);
    final extent = duoRailCapsuleExtent(duoRailBottomControls);
    // The Bestie slot is the capsule's last 48 points above its 6-point margin,
    // and the agent shortcut and its voice pill are the same 48 points.
    const controlSize = 48.0;
    const slotBottom = 6.0;
    return AgentNavigationOverlay(
      active: overlay,
      controlSize: controlSize,
      onRecordingChanged: (value) {
        recording.value = value;
        onRecordingChanged?.call(value);
      },
      railLink: link,
      child: Transform.translate(
        offset: Offset(
          IosNavigationMetrics.of(
                context,
              ).railCenter(MediaQuery.sizeOf(context), strip.rect.center.dx) -
              strip.rect.center.dx,
          0,
        ),
        child: SizedBox(
          key: const ValueKey('messages-rail-controls'),
          width: strip.width,
          height: extent,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              Positioned.fill(
                child: IosNavigationBar(
                  title: '',
                  standaloneControl: true,
                  actions: actions,
                ),
              ),
              // The voice pill grows leftward from the control, over Home.
              Positioned(
                bottom: slotBottom,
                right: strip.width / 2 - controlSize / 2,
                width: 256,
                height: controlSize,
                child: OverflowBox(
                  alignment: Alignment.centerRight,
                  minWidth: 0,
                  maxWidth: 256,
                  child: Align(
                    widthFactor: 1,
                    alignment: Alignment.centerRight,
                    child: NavigationGlassGroup(child: agent),
                  ),
                ),
              ),
              // Native controls cannot be dimmed from here, so while Bestie
              // records, Search and New are made inert instead, as the bar's
              // own controls are. Changing their native actions would rebuild
              // the capsule and cancel the hold, and the Bestie control that
              // owns it stays reachable below.
              if (recording.value)
                Positioned(
                  key: const ValueKey('messages-rail-inert'),
                  left: 0,
                  right: 0,
                  top: 0,
                  // Everything above the Bestie slot and its 6-point margin.
                  height: extent - controlSize - 6,
                  child: const AbsorbPointer(child: SizedBox.expand()),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
