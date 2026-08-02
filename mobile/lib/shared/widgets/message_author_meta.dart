import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// A consistent inline author row for message-oriented surfaces.
class MessageAuthorMeta extends StatelessWidget {
  /// Primary author name shown at the start of the row.
  final String displayName;

  /// Optional secondary username, hidden when blank or equal to [displayName].
  final String? username;

  /// Timestamp label shown after the author metadata.
  final String timestamp;

  /// Color applied to [displayName].
  final Color nameColor;

  /// Color applied to the username, separator, and [timestamp].
  final Color metadataColor;

  /// Optional callback invoked when [displayName] is tapped.
  final VoidCallback? onAuthorTap;

  /// Optional key assigned to the display-name text.
  final Key? displayNameKey;

  /// Optional key assigned to the username text.
  final Key? usernameKey;

  /// Optional key assigned to the timestamp text.
  final Key? timestampKey;

  /// Base text style for [displayName], with [nameColor] applied.
  final TextStyle nameStyle;

  /// Base text style for secondary metadata, with [metadataColor] applied.
  final TextStyle metadataStyle;

  /// Creates an inline author row with optional username and tap handling.
  const MessageAuthorMeta({
    super.key,
    required this.displayName,
    required this.timestamp,
    required this.nameColor,
    required this.metadataColor,
    this.username,
    this.onAuthorTap,
    this.displayNameKey,
    this.usernameKey,
    this.timestampKey,
    this.nameStyle = messageUsernameTextStyle,
    this.metadataStyle = messageMetadataTextStyle,
  });

  @override
  Widget build(BuildContext context) {
    final normalizedUsername = username?.trim();
    final showUsername =
        normalizedUsername != null &&
        normalizedUsername.isNotEmpty &&
        normalizedUsername != displayName.trim();
    final resolvedNameStyle = nameStyle.copyWith(color: nameColor);
    final resolvedMetadataStyle = metadataStyle.copyWith(color: metadataColor);

    Widget authorName = Text(
      displayName,
      key: displayNameKey,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: resolvedNameStyle,
    );
    if (onAuthorTap != null) {
      authorName = GestureDetector(onTap: onAuthorTap, child: authorName);
    }

    return LayoutBuilder(
      builder: (context, constraints) {
        final metadataMaxWidth = constraints.hasBoundedWidth
            ? constraints.maxWidth / (showUsername ? 3 : 2)
            : double.infinity;

        return Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            Flexible(child: authorName),
            if (showUsername) ...[
              const SizedBox(width: Grid.half),
              ConstrainedBox(
                constraints: BoxConstraints(maxWidth: metadataMaxWidth),
                child: Text(
                  normalizedUsername,
                  key: usernameKey,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: resolvedMetadataStyle,
                ),
              ),
            ],
            const SizedBox(width: Grid.half),
            Text('·', style: resolvedMetadataStyle),
            const SizedBox(width: Grid.half),
            ConstrainedBox(
              constraints: BoxConstraints(maxWidth: metadataMaxWidth),
              child: Text(
                timestamp,
                key: timestampKey,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: resolvedMetadataStyle,
              ),
            ),
          ],
        );
      },
    );
  }
}
