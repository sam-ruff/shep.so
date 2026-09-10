import 'package:flutter/material.dart';
import '../model/workspace.dart';
import 'controls.dart';
import 'icons.dart';
import 'theme.dart';

/// Shared by the inbox and pushed reader; errors remain separate from expiry.
class MailActionBanner extends StatelessWidget {
  const MailActionBanner({super.key, required this.workspace});
  final Workspace workspace;
  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    final label = workspace.actionNotice;
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (workspace.notice != null && workspace.notice != 'Message updated')
          Semantics(
            liveRegion: true,
            child: NoticeBar(child: Text(workspace.notice!)),
          ),
        if (label != null)
          Semantics(
            liveRegion: true,
            child: ToastCard(
              trailing: [
                if (workspace.undo != null)
                  TextButton(
                    onPressed: workspace.undo,
                    child: const Text('Undo'),
                  ),
                if (workspace.moves.visible)
                  IconButton(
                    tooltip: 'Dismiss move notification',
                    onPressed: workspace.moves.dismiss,
                    icon: const ShepIcon('close', size: 18),
                  ),
              ],
              child: Text(label),
            ),
          ),
        if (workspace.undoFailures.isNotEmpty)
          Semantics(
            liveRegion: true,
            child: Material(
              color: c.errorSurface,
              child: Padding(
                padding: const EdgeInsets.only(left: 16, right: 4),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Row(
                      children: [
                        Expanded(
                          child: Text(
                            '${workspace.undoFailures.length} ${workspace.undoFailures.length == 1 ? 'move needs' : 'moves need'} review.',
                            style: TextStyle(color: c.text),
                          ),
                        ),
                        IconButton(
                          tooltip: 'Dismiss Undo errors',
                          onPressed: workspace.dismissUndoFailures,
                          icon: const ShepIcon('close', size: 18),
                        ),
                      ],
                    ),
                    Wrap(
                      alignment: WrapAlignment.end,
                      children: [
                        if (workspace.undoFailures.any(
                          (r) => !r.restoreCommitted,
                        ))
                          TextButton(
                            onPressed: workspace.retryUndos,
                            child: const Text('Retry Undo'),
                          ),
                        if (workspace.undoFailures.any(
                          (r) => r.restoreCommitted,
                        ))
                          TextButton(
                            onPressed: workspace.refreshRestored,
                            child: const Text('Refresh restored mail'),
                          ),
                      ],
                    ),
                  ],
                ),
              ),
            ),
          ),
      ],
    );
  }
}
