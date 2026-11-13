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
class _LinkTextRecognizer extends TapGestureRecognizer {
  VoidCallback? onHold;
  Timer? _holdTimer;
  bool _held = false;

  @override
  void addAllowedPointer(PointerDownEvent event) {
    // Only the pointer that starts a gesture times the hold; a second finger
    // joining it must not restart the first finger's clock.
    final startsGesture = state == GestureRecognizerState.ready;
    super.addAllowedPointer(event);
    if (!startsGesture) return;
    _held = false;
    _holdTimer?.cancel();
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
    _holdTimer?.cancel();
    if (!_held) super.handleTapUp(down: down, up: up);
  }

  @override
  void handleTapCancel({
    required PointerDownEvent down,
    PointerCancelEvent? cancel,
    required String reason,
  }) {
    _holdTimer?.cancel();
    if (!_held) {
      super.handleTapCancel(down: down, cancel: cancel, reason: reason);
    }
  }

  // A scroll can take the pointer before the tap-down deadline, when no tap
  // cancel is sent, so the hold timer is also stopped here.
  @override
  void rejectGesture(int pointer) {
    if (pointer == primaryPointer) {
      _holdTimer?.cancel();
    }
    super.rejectGesture(pointer);
  }

  @override
  void dispose() {
    _holdTimer?.cancel();
    super.dispose();
  }
}

/// One [_LinkTextRecognizer] per link in a message's text, kept across
/// rebuilds and disposed when the text changes or the message goes away.
class _LinkTextRecognizers {
  final _byLink = <String, _LinkTextRecognizer>{};

  _LinkTextRecognizer forLink(String url, String label) =>
      _byLink.putIfAbsent('$url\n$label', _LinkTextRecognizer.new);

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
