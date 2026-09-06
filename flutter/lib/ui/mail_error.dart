import 'package:flutter/material.dart';
import '../model/workspace.dart';

/// Used inside each mail route so a pushed reader cannot hide an action error.
class MailErrorBanner extends StatelessWidget {
  const MailErrorBanner({super.key, required this.workspace});
  final Workspace workspace;

  @override
  Widget build(BuildContext context) {
    final error = workspace.error;
    if (error == null) return const SizedBox.shrink();
    final scheme = Theme.of(context).colorScheme;
    return Semantics(
      liveRegion: true,
      child: Material(
        color: scheme.errorContainer,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 4, 8),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  error,
                  style: TextStyle(
                    fontSize: 12,
                    color: scheme.onErrorContainer,
                  ),
                ),
              ),
              if (workspace.retry != null)
                TextButton(
                  onPressed: workspace.retry,
                  child: const Text('Retry'),
                ),
              IconButton(
                tooltip: 'Dismiss error',
                onPressed: workspace.clearError,
                icon: const Icon(Icons.close, size: 18),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
