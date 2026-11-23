import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// Retains readable history while surfacing a failed head refresh.
class ForumRefreshError extends StatelessWidget {
  final Object? error;
  final VoidCallback onRetry;
  const ForumRefreshError({
    super.key,
    required this.error,
    required this.onRetry,
  });

  @override
  Widget build(BuildContext context) => error == null
      ? const SizedBox.shrink()
      : Padding(
          padding: const EdgeInsets.all(Grid.xs),
          child: Column(
            children: [
              const Text(
                'Could not refresh. Loaded history may be incomplete.',
              ),
              TextButton(
                onPressed: onRetry,
                child: const Text('Retry refresh'),
              ),
            ],
          ),
        );
}
