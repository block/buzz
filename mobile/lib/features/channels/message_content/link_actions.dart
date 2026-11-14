part of '../message_content.dart';

/// Wraps a rendered link so a hold opens [_showLinkActions] instead of the
/// surrounding message's menu.
///
/// The recognizer shares [iosMessageLongPressDuration] with the message row.
/// Pointer events reach the deepest recognizer first, so at equal deadlines
/// the link's timer fires and wins the arena before the message row's does,
/// the same way reaction pills claim their own hold.
class _LinkLongPress extends HookWidget {
  final String url;

  /// Opens the link; null hides Open for destinations nothing can open.
  final VoidCallback? onOpen;

  /// Tint painted behind the link while a finger is down on it.
  final Color highlightColor;
  final Widget child;

  const _LinkLongPress({
    required this.url,
    required this.onOpen,
    required this.highlightColor,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    // Only this link rebuilds on press; the surrounding markdown does not.
    final pressed = useState(false);
    void open() => _showLinkActions(context, url: url, onOpen: onOpen);
    void release() => pressed.value = false;

    return Semantics(
      onLongPress: open,
      child: RawGestureDetector(
        gestures: {
          LongPressGestureRecognizer:
              GestureRecognizerFactoryWithHandlers<LongPressGestureRecognizer>(
                () => LongPressGestureRecognizer(
                  duration: defaultTargetPlatform == TargetPlatform.iOS
                      ? iosMessageLongPressDuration
                      : null,
                ),
                (recognizer) {
                  // Fires on every pointer down, so a tap highlights too; a
                  // tap or scroll rejects the hold and clears it via cancel.
                  recognizer
                    ..onLongPressDown = (_) {
                      pressed.value = true;
                    }
                    ..onLongPressCancel = release
                    ..onLongPress = open
                    ..onLongPressUp = release;
                },
              ),
        },
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: pressed.value ? highlightColor : null,
            borderRadius: BorderRadius.circular(Radii.xs),
          ),
          child: child,
        ),
      ),
    );
  }
}

/// Tap and hold for a link drawn as text, so it wraps with the sentence.
///
/// A span holds one recognizer, so this one does both: a tap opens the link
/// and a hold of [iosMessageLongPressDuration] opens [_showLinkActions]. The
/// hold timer starts at pointer down, the same moment the message row's long
/// press starts its own, and the paragraph sees the pointer first, so the
/// link wins the hold the same way [_LinkLongPress] does.
///
/// While a finger is down it is the message's [_pressed] link, which
/// [_LinkPressHighlight] tints with [highlightColor].
class _LinkTextRecognizer extends TapGestureRecognizer {
  _LinkTextRecognizer(this._pressed);

  final ValueNotifier<_LinkTextRecognizer?> _pressed;
  VoidCallback? onHold;
  Color? highlightColor;
  Timer? _holdTimer;
  bool _held = false;

  void _release() {
    _holdTimer?.cancel();
    if (_pressed.value == this) _pressed.value = null;
  }

  @override
  void addAllowedPointer(PointerDownEvent event) {
    // Only the pointer that starts a gesture times the hold; a second finger
    // joining it must not restart the first finger's clock.
    final startsGesture = state == GestureRecognizerState.ready;
    super.addAllowedPointer(event);
    if (!startsGesture) return;
    _held = false;
    _holdTimer?.cancel();
    _pressed.value = this;
    _holdTimer = Timer(
      defaultTargetPlatform == TargetPlatform.iOS
          ? iosMessageLongPressDuration
          : kLongPressTimeout,
      () {
        _held = true;
        resolve(GestureDisposition.accepted);
        onHold?.call();
      },
    );
  }

  @override
  void handleTapUp({
    required PointerDownEvent down,
    required PointerUpEvent up,
  }) {
    _release();
    if (!_held) super.handleTapUp(down: down, up: up);
  }

  @override
  void handleTapCancel({
    required PointerDownEvent down,
    PointerCancelEvent? cancel,
    required String reason,
  }) {
    _release();
    if (!_held) {
      super.handleTapCancel(down: down, cancel: cancel, reason: reason);
    }
  }

  // A scroll can take the pointer before the tap-down deadline, when no tap
  // cancel is sent, so the hold timer and tint are also stopped here.
  @override
  void rejectGesture(int pointer) {
    if (pointer == primaryPointer) _release();
    super.rejectGesture(pointer);
  }

  @override
  void dispose() {
    _release();
    super.dispose();
  }
}

/// One [_LinkTextRecognizer] per link in a message's text, kept across
/// rebuilds and disposed when the text changes or the message goes away.
class _LinkTextRecognizers {
  _LinkTextRecognizers(this._pressed);

  final ValueNotifier<_LinkTextRecognizer?> _pressed;
  final _byLink = <String, _LinkTextRecognizer>{};

  _LinkTextRecognizer forLink(String url, String label) =>
      _byLink.putIfAbsent('$url\n$label', () => _LinkTextRecognizer(_pressed));

  void dispose() {
    for (final recognizer in _byLink.values) {
      recognizer.dispose();
    }
    _byLink.clear();
  }
}

/// [spans] with [recognizer] on every text run, so the whole label is one
/// link. Inline code keeps its chip rendering. Other package span types are
/// left as they are.
List<InlineSpan> _withLinkRecognizer(
  List<InlineSpan> spans,
  GestureRecognizer recognizer,
) => [
  for (final span in spans)
    if (span is CodeTextSpan)
      span.children == null
          ? CodeTextSpan(
              text: span.text ?? '',
              codeStyle: span.codeStyle,
              style: span.style,
              recognizer: span.recognizer ?? recognizer,
              mouseCursor: span.mouseCursor,
              semanticsLabel: span.semanticsLabel,
            )
          : CodeTextSpan.revealing(
              children: _withLinkRecognizer(span.children!, recognizer),
              codeStyle: span.codeStyle,
              style: span.style,
              recognizer: span.recognizer ?? recognizer,
              mouseCursor: span.mouseCursor,
              semanticsLabel: span.semanticsLabel,
            )
    else if (span.runtimeType == TextSpan)
      TextSpan(
        text: (span as TextSpan).text,
        style: span.style,
        children: span.children == null
            ? null
            : _withLinkRecognizer(span.children!, recognizer),
        recognizer: span.recognizer ?? recognizer,
        semanticsLabel: span.semanticsLabel,
        locale: span.locale,
        spellOut: span.spellOut,
      )
    else
      span,
];

/// Link-specific actions: open, copy and share the destination.
void _showLinkActions(
  BuildContext context, {
  required String url,
  required VoidCallback? onOpen,
}) {
  unawaited(HapticFeedback.mediumImpact());
  final uri = Uri.tryParse(url);
  // Web links share as URLs so the share sheet can preview them; any other
  // link (buzz://, mailto:) shares as its text.
  final isWebLink =
      uri != null && (uri.scheme == 'http' || uri.scheme == 'https');
  unawaited(
    showBuzzModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      title: 'Link',
      builder: (sheetContext) => SafeArea(
        // A long URL and large text can outgrow a short landscape sheet.
        child: SingleChildScrollView(
          child: IconTheme.merge(
            data: const IconThemeData(size: 22),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(
                Grid.gutter,
                0,
                Grid.gutter,
                Grid.xs,
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Padding(
                    padding: const EdgeInsets.only(bottom: Grid.xs),
                    child: Text(
                      url,
                      maxLines: 3,
                      overflow: TextOverflow.ellipsis,
                      style: sheetContext.textTheme.bodySmall?.copyWith(
                        color: sheetContext.colors.onSurfaceVariant,
                      ),
                    ),
                  ),
                  SheetActionSection(
                    children: [
                      if (onOpen != null)
                        ListTile(
                          leading: const Icon(BuzzIcons.externalLink),
                          title: const Text('Open link'),
                          onTap: () {
                            Navigator.of(sheetContext).pop();
                            onOpen();
                          },
                        ),
                      ListTile(
                        leading: const Icon(BuzzIcons.copy),
                        title: const Text('Copy link'),
                        onTap: () {
                          Navigator.of(sheetContext).pop();
                          unawaited(
                            copyToClipboard(
                              context,
                              url,
                              message: 'Link copied',
                            ),
                          );
                        },
                      ),
                      ListTile(
                        leading: const Icon(BuzzIcons.share2),
                        title: const Text('Share link'),
                        onTap: () {
                          final renderBox =
                              context.findRenderObject() as RenderBox?;
                          final shareOrigin = renderBox == null
                              ? null
                              : renderBox.localToGlobal(Offset.zero) &
                                    renderBox.size;
                          Navigator.of(sheetContext).pop();
                          unawaited(
                            SharePlus.instance.share(
                              ShareParams(
                                uri: isWebLink ? uri : null,
                                text: isWebLink ? null : url,
                                sharePositionOrigin: shareOrigin,
                              ),
                            ),
                          );
                        },
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    ),
  );
}

/// Tints the [pressed] link's text behind [child], so a press marks the link
/// the way [_LinkLongPress] does for chips.
///
/// Paint-only: a press repaints this box and rebuilds no markdown.
class _LinkPressHighlight extends SingleChildRenderObjectWidget {
  const _LinkPressHighlight({required this.pressed, required super.child});

  final ValueListenable<_LinkTextRecognizer?> pressed;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderLinkPressHighlight(pressed);

  @override
  void updateRenderObject(
    BuildContext context,
    _RenderLinkPressHighlight renderObject,
  ) => renderObject.pressed = pressed;
}

class _RenderLinkPressHighlight extends RenderProxyBox {
  _RenderLinkPressHighlight(this._pressed);

  ValueListenable<_LinkTextRecognizer?> _pressed;
  set pressed(ValueListenable<_LinkTextRecognizer?> value) {
    if (value == _pressed) return;
    if (attached) {
      _pressed.removeListener(markNeedsPaint);
      value.addListener(markNeedsPaint);
    }
    _pressed = value;
    markNeedsPaint();
  }

  @override
  void attach(PipelineOwner owner) {
    super.attach(owner);
    _pressed.addListener(markNeedsPaint);
  }

  @override
  void detach() {
    _pressed.removeListener(markNeedsPaint);
    super.detach();
  }

  @override
  void paint(PaintingContext context, Offset offset) {
    final link = _pressed.value;
    final color = link?.highlightColor;
    final root = child;
    if (link != null && color != null && root != null) {
      final paint = Paint()..color = color;
      void visit(RenderObject node) {
        if (node is RenderParagraph) {
          final transform = node.getTransformTo(this);
          for (final range in _linkRanges(node.text, link)) {
            final boxes = node.getBoxesForSelection(
              TextSelection(baseOffset: range.start, extentOffset: range.end),
            );
            for (final box in boxes) {
              final rect = MatrixUtils.transformRect(transform, box.toRect());
              context.canvas.drawRRect(
                RRect.fromRectAndRadius(
                  rect.shift(offset),
                  const Radius.circular(Radii.xs),
                ),
                paint,
              );
            }
          }
        }
        node.visitChildren(visit);
      }

      visit(root);
    }
    super.paint(context, offset);
  }
}

/// The runs of [text] that carry [link], merged where they touch.
List<TextRange> _linkRanges(InlineSpan text, GestureRecognizer link) {
  final ranges = <TextRange>[];
  var offset = 0;
  text.visitChildren((span) {
    final length = switch (span) {
      TextSpan(:final text?) => text.length,
      PlaceholderSpan() => 1,
      _ => 0,
    };
    if (length > 0 && span is TextSpan && span.recognizer == link) {
      if (ranges.isNotEmpty && ranges.last.end == offset) {
        ranges.last = TextRange(start: ranges.last.start, end: offset + length);
      } else {
        ranges.add(TextRange(start: offset, end: offset + length));
      }
    }
    offset += length;
    return true;
  });
  return ranges;
}
