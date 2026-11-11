import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// A visible completeness signal and retry affordance for a forum window.
class ForumPagination extends StatelessWidget {
  final bool hasMore;
  final bool isLoading;
  final Object? error;
  final String subject;
  final VoidCallback onLoadMore;

  const ForumPagination({
    super.key,
    required this.hasMore,
    required this.isLoading,
    required this.error,
    required this.subject,
    required this.onLoadMore,
  });

  @override
  Widget build(BuildContext context) {
    if (!hasMore) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.all(Grid.xs),
      child: Column(
        children: [
          Text(
            error == null
                ? 'Older $subject are available.'
                : 'Could not load older $subject. History is incomplete.',
            textAlign: TextAlign.center,
          ),
          TextButton(
            onPressed: isLoading ? null : onLoadMore,
            child: Text(
              isLoading
                  ? 'Loading $subject…'
                  : error == null
                  ? 'Load older $subject'
                  : 'Retry older $subject',
            ),
          ),
        ],
      ),
    );
  }
}
