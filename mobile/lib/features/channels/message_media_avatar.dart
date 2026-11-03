import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'message_gallery.dart';
import 'message_media.dart';
import 'message_content/link_normalizer.dart';

import '../../shared/theme/message_typography.dart';

/// Retains gallery displacement across caption edits, resetting for new images.
ValueNotifier<double> useMessageMediaOffset(
  String content,
  List<List<String>> tags,
) {
  final gallery = extractTrailingImageGallery(normalizeBareLinks(content), {
    for (final entry in parseImetaTags(tags).entries)
      normalizeMarkdownDestination(entry.key): entry.value,
  });
  final signature = gallery?.items.map((item) => item.url).join('\u0000');
  final offset = useValueNotifier(0.0);
  useEffect(() {
    offset.value = 0;
    return null;
  }, [signature]);
  return offset;
}

/// Reports gallery displacement so the author clears the overflowing images.
class MessageGalleryScrollNotification extends Notification {
  /// Creates a displacement update relative to the first image.
  const MessageGalleryScrollNotification(this.offset);

  /// Gallery displacement in logical pixels.
  final double offset;
}

/// Fades the author away as the gallery moves into its gutter.
/// Remains hidden on later images and returns when the first image returns.
class MessageMediaAvatar extends StatelessWidget {
  /// Connects the author presentation to its message's gallery position.
  const MessageMediaAvatar({
    super.key,
    required this.offset,
    required this.child,
  });

  /// Gallery displacement relative to the first image.
  final ValueListenable<double> offset;

  /// Author avatar and its profile action.
  final Widget child;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<double>(
    valueListenable: offset,
    child: child,
    builder: (context, offset, child) {
      final visibility = 1 - (offset / messageAvatarSize).clamp(0.0, 1.0);
      return IgnorePointer(
        ignoring: visibility < 1,
        child: ExcludeSemantics(
          excluding: visibility < 1,
          child: Opacity(
            opacity: visibility,
            child: Transform.scale(
              scale: MediaQuery.disableAnimationsOf(context) ? 1 : visibility,
              child: child,
            ),
          ),
        ),
      );
    },
  );
}
