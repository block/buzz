part of '../compose_bar.dart';

enum _InputTool { keyboard, attachments }

class _ComposerInputTray extends HookWidget {
  const _ComposerInputTray({
    required this.tool,
    required this.focusNode,
    required this.enabled,
    required this.attachments,
    required this.attachmentSurface,
    required this.suggestions,
    required this.onDismissSuggestions,
    required this.child,
  });
  final ValueNotifier<_InputTool?> tool;
  final FocusNode focusNode;
  final bool enabled;
  final VoidCallback onDismissSuggestions;
  final Widget? suggestions;
  final Widget child;
  final Widget Function(_AttachmentSurface) attachments;
  final ValueNotifier<_AttachmentSurface> attachmentSurface;

  @override
  Widget build(BuildContext context) {
    final portal = useMemoized(OverlayPortalController.new);
    useListenable(focusNode);
    useListenable(tool);
    final visible =
        enabled &&
        (tool.value == _InputTool.attachments || suggestions != null) &&
        (ModalRoute.of(context)?.isCurrent ?? true);
    final expanded =
        tool.value == _InputTool.attachments || suggestions != null;
    void type() {
      tool.value = null;
      attachmentSurface.value = _AttachmentSurface.closed;
      onDismissSuggestions();
    }

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        if (visible) {
          portal.show();
        } else {
          portal.hide();
          tool.value = null;
        }
      });
      return null;
    }, [visible]);

    return OverlayPortal.overlayChildLayoutBuilder(
      controller: portal,
      overlayLocation: OverlayChildLocation.rootOverlay,
      overlayChildBuilder: (context, info) {
        final origin = MatrixUtils.transformPoint(
          info.childPaintTransform,
          Offset.zero,
        );
        final availableWidth = math.max(0.0, info.childSize.width);
        final attachmentOpen = tool.value == _InputTool.attachments;
        final content = attachmentOpen
            ? attachments(attachmentSurface.value)
            : suggestions?.key == const ValueKey('no-suggestions')
            ? const Padding(
                padding: EdgeInsets.all(24),
                child: Text('No matching results'),
              )
            : suggestions ?? const SizedBox.shrink();
        final width = math.min(
          availableWidth,
          attachmentOpen && attachmentSurface.value == _AttachmentSurface.menu
              ? 280.0
              : expanded
              ? 440.0
              : 560.0,
        );
        final maxHeight = math.max(
          48.0,
          origin.dy - 8 - MediaQuery.paddingOf(context).top,
        );
        return Positioned(
          left: origin.dx,
          bottom: info.overlaySize.height - origin.dy + 8,
          width: availableWidth,
          child: Align(
            alignment: Alignment.bottomLeft,
            heightFactor: 1,
            child: TapRegion(
              groupId: portal,
              consumeOutsideTaps: expanded,
              onTapOutside: expanded ? (_) => type() : null,
              child: TextFieldTapRegion(
                child: _ComposerGlassScope(
                  child: Material(
                    type: MaterialType.transparency,
                    child: _ComposerToolStripGlass(
                      canvasSize: Size(availableWidth, maxHeight),
                      phase: attachmentOpen
                          ? attachmentSurface.value.name
                          : suggestions != null
                          ? 'suggestions'
                          : 'strip',
                      child: ConstrainedBox(
                        constraints: BoxConstraints(
                          maxHeight: maxHeight,
                          minWidth: width,
                          maxWidth: width,
                        ),
                        child:
                            attachmentOpen &&
                                attachmentSurface.value !=
                                    _AttachmentSurface.menu
                            ? SizedBox(
                                height: math.min(320, maxHeight),
                                child: content,
                              )
                            : content,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        );
      },
      // Composer controls belong to the tray, so a tap on them reaches its
      // target instead of being consumed as a dismissal.
      child: TapRegion(groupId: portal, child: child),
    );
  }
}

/// Suppresses nested cards when content is hosted in the shared glass surface.
class _ComposerGlassScope extends InheritedWidget {
  const _ComposerGlassScope({required super.child});
  static bool of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_ComposerGlassScope>() != null;
  @override
  bool updateShouldNotify(_ComposerGlassScope oldWidget) => false;
}

/// Native glass identities share a canvas large enough for both transition states.
class _ComposerToolStripGlass extends HookWidget {
  const _ComposerToolStripGlass({
    required this.child,
    required this.canvasSize,
    required this.phase,
  });
  final Widget child;
  final Size canvasSize;
  final String phase;

  @override
  Widget build(BuildContext context) {
    final channel = useRef<MethodChannel?>(null);
    final measured = useRef<Size?>(null);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final contentReveal = useAnimationController(
      initialValue: 1,
      duration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 240),
    );
    final previousPhase = useRef(phase);
    useEffect(() {
      if (reducedMotion) {
        contentReveal.value = 1;
      } else if (previousPhase.value != phase) {
        contentReveal.forward(from: 0);
      }
      previousPhase.value = phase;
      return null;
    }, [phase, reducedMotion]);
    Future<void> update(Size size) async {
      measured.value = size;
      try {
        await channel.value?.invokeMethod<void>('geometry', {
          'width': size.width,
          'height': size.height,
          'phase': phase,
          'reducedMotion': reducedMotion,
        });
      } on MissingPluginException {
        // Platform view registration is absent in widget tests.
      } on PlatformException {
        // A disposed native view no longer needs geometry updates.
      }
    }

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted && measured.value != null) {
          unawaited(update(measured.value!));
        }
      });
      return null;
    }, [phase, reducedMotion]);
    return Stack(
      clipBehavior: Clip.none,
      alignment: Alignment.bottomLeft,
      children: [
        Positioned(
          left: 0,
          bottom: 0,
          width: canvasSize.width,
          height: canvasSize.height,
          child: IgnorePointer(
            child: ExcludeSemantics(
              child: UiKitView(
                key: ValueKey(context.theme.brightness),
                viewType: 'buzz/composer_tool_strip_glass',
                creationParams: {'brightness': context.theme.brightness.name},
                creationParamsCodec: const StandardMessageCodec(),
                onPlatformViewCreated: (id) {
                  channel.value = MethodChannel(
                    'buzz/composer_tool_strip_glass/$id',
                  );
                  if (measured.value != null) {
                    unawaited(update(measured.value!));
                  }
                },
              ),
            ),
          ),
        ),
        ClipRRect(
          borderRadius: BorderRadius.circular(24),
          child: AnimatedSize(
            duration: reducedMotion
                ? Duration.zero
                : const Duration(milliseconds: 180),
            alignment: Alignment.bottomLeft,
            curve: Curves.easeInOutCubic,
            child: _GlassContentSize(
              onSize: (size) {
                measured.value = size;
                WidgetsBinding.instance.addPostFrameCallback((_) {
                  if (context.mounted) unawaited(update(measured.value!));
                });
              },
              child: FadeTransition(
                opacity: contentReveal.drive(
                  CurveTween(curve: const Interval(0.85, 1)),
                ),
                child: AnimatedBuilder(
                  animation: contentReveal,
                  child: child,
                  builder: (_, child) => IgnorePointer(
                    ignoring: contentReveal.value < 0.85,
                    child: child,
                  ),
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _GlassContentSize extends SingleChildRenderObjectWidget {
  const _GlassContentSize({required this.onSize, required super.child});
  final ValueChanged<Size> onSize;
  @override
  RenderObject createRenderObject(BuildContext context) =>
      _GlassContentSizeRender(onSize);
  @override
  void updateRenderObject(
    BuildContext context,
    _GlassContentSizeRender renderObject,
  ) {
    renderObject.onSize = onSize;
  }
}

class _GlassContentSizeRender extends RenderProxyBox {
  _GlassContentSizeRender(this.onSize);
  ValueChanged<Size> onSize;
  Size? reported;
  @override
  void performLayout() {
    super.performLayout();
    if (reported == size) return;
    reported = size;
    onSize(size);
  }
}
