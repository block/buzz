import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Retains each destination while moving between the two visible pages.
class RetainedPageTransition extends HookWidget {
  const RetainedPageTransition({
    super.key,
    required this.index,
    required this.children,
  });

  final int index;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final reduced = MediaQuery.disableAnimationsOf(context);
    final controller = useAnimationController(
      duration: const Duration(milliseconds: 240),
      initialValue: 1,
    );
    final previous = useRef(index);
    final current = useRef(index);
    useEffect(() {
      if (current.value != index) {
        previous.value = current.value;
        current.value = index;
        if (reduced) {
          controller.value = 1;
        } else {
          controller.forward(from: 0);
        }
      }
      return null;
    }, [index, reduced]);
    final progress = Curves.easeOutCubic.transform(useAnimation(controller));
    final direction = index >= previous.value ? 1.0 : -1.0;
    return ClipRect(
      child: Stack(
        fit: StackFit.expand,
        children: [
          for (var i = 0; i < children.length; i++)
            Offstage(
              offstage: i != index && (i != previous.value || progress == 1),
              child: TickerMode(
                enabled: i == index || (i == previous.value && progress < 1),
                child: IgnorePointer(
                  ignoring: i != index,
                  child: ExcludeFocus(
                    excluding: i != index,
                    child: ExcludeSemantics(
                      excluding: i != index,
                      child: Transform.translate(
                        offset: Offset(
                          reduced
                              ? 0
                              : 24 *
                                    direction *
                                    (i == index ? 1 - progress : -progress),
                          0,
                        ),
                        child: Opacity(
                          opacity: i == index ? progress : 1 - progress,
                          child: children[i],
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}
