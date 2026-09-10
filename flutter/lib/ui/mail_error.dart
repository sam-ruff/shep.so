import 'package:flutter/material.dart';
import '../model/workspace.dart';
import 'controls.dart';
import 'icons.dart';

/// Used inside each mail route so a pushed reader cannot hide an action error.
class MailErrorBanner extends StatelessWidget {
  const MailErrorBanner({super.key, required this.workspace});
  final Workspace workspace;

  @override
  Widget build(BuildContext context) {
    final error = workspace.error;
    if (error == null) return const SizedBox.shrink();
    return Semantics(
      liveRegion: true,
      child: NoticeBar(
        error: true,
        trailing: [
          if (workspace.retry != null)
            TextButton(onPressed: workspace.retry, child: const Text('Retry')),
          IconButton(
            tooltip: 'Dismiss error',
            onPressed: workspace.clearError,
            icon: const ShepIcon('close', size: 18),
          ),
        ],
        child: Text(error),
      ),
    );
  }
}
