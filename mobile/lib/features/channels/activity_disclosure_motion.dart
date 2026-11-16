import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// Keeps the revealed surface rounded and populated throughout closing.
class ActivityDisclosureMotion extends StatefulWidget {
  const ActivityDisclosureMotion({
    super.key,
    required this.expanded,
    required this.onToggle,
    required this.child,
  });

  final bool expanded;
  final VoidCallback onToggle;
  final Widget child;

  @override
  State<ActivityDisclosureMotion> createState() =>
      _ActivityDisclosureMotionState();
}

class _ActivityDisclosureMotionState extends State<ActivityDisclosureMotion> {
  final _contentKey = GlobalKey();
  Widget? _closingContent;
  Size? _closingSize;

  @override
  void didUpdateWidget(ActivityDisclosureMotion oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.expanded) {
      _closingContent = null;
      _closingSize = null;
    } else if (oldWidget.expanded && !MediaQuery.disableAnimationsOf(context)) {
      final box = _contentKey.currentContext?.findRenderObject();
      if (box is RenderBox && box.hasSize) {
        _closingContent = oldWidget.child;
        _closingSize = box.size;
      }
    }
  }

  void _finishClosing() {
    if (!widget.expanded && _closingContent != null && mounted) {
      setState(() {
        _closingContent = null;
        _closingSize = null;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final reduced = MediaQuery.disableAnimationsOf(context);
    final closing = !reduced && _closingContent != null;
    final content = Stack(
      clipBehavior: Clip.none,
      children: [
        Opacity(
          opacity: closing ? 0 : 1,
          child: KeyedSubtree(key: _contentKey, child: widget.child),
        ),
        if (closing)
          Positioned(
            top: 0,
            left: 0,
            width: _closingSize!.width,
            height: _closingSize!.height,
            child: IgnorePointer(
              child: ExcludeSemantics(child: _closingContent!),
            ),
          ),
      ],
    );
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      excludeFromSemantics: true,
      onTap: widget.expanded ? null : widget.onToggle,
      child: AnimatedContainer(
        duration: reduced ? Duration.zero : const Duration(milliseconds: 220),
        curve: Curves.easeOutCubic,
        decoration: BoxDecoration(
          color: widget.expanded || closing
              ? context.colors.secondaryContainer
              : Colors.transparent,
          borderRadius: BorderRadius.circular(Radii.container),
        ),
        clipBehavior: Clip.antiAlias,
        child: reduced
            ? content
            : AnimatedSize(
                duration: Duration(milliseconds: widget.expanded ? 220 : 180),
                curve: Curves.easeOutCubic,
                alignment: Alignment.topLeft,
                onEnd: _finishClosing,
                child: content,
              ),
      ),
    );
  }
}
