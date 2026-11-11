import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// A visible completeness signal and retry affordance for a forum window.
class ForumPagination extends StatelessWidget {
  final bool hasMore;
  final bool isLoading;
  final Object? error;
  final String subject;
  final VoidCallback onLoadMore;
  final bool isUnconfirmed;

  const ForumPagination({
    super.key,
    required this.hasMore,
    required this.isLoading,
    required this.error,
    required this.subject,
    required this.onLoadMore,
    this.isUnconfirmed = false,
  });

  @override
  Widget build(BuildContext context) {
    if (!hasMore) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.all(Grid.xs),
      child: Column(
        children: [
          Text(
            isUnconfirmed
                ? error == null
                      ? 'There may be older $subject'
                      : 'History may be incomplete. The relay could not continue this history check.'
                : error == null
                ? 'Older $subject are available.'
                : 'Could not load older $subject. History is incomplete.',
            textAlign: TextAlign.center,
          ),
          TextButton(
            onPressed: isLoading ? null : onLoadMore,
            child: Text(
              isUnconfirmed
                  ? isLoading
                        ? 'Checking history…'
                        : error == null
                        ? 'Check for older $subject'
                        : 'Check history again'
                  : isLoading
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
