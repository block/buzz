import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// A consistent inline author row for message-oriented surfaces.
class MessageAuthorMeta extends StatelessWidget {
  final String displayName;
  final String? username;
  final String timestamp;
  final Color nameColor;
  final Color metadataColor;
  final VoidCallback? onAuthorTap;
  final Key? displayNameKey;
  final Key? usernameKey;
  final Key? timestampKey;
  final TextStyle nameStyle;
  final TextStyle metadataStyle;

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
            Expanded(child: authorName),
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
