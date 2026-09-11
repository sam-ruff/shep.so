import 'dart:async';
import 'package:flutter/material.dart';
import '../data/groups.dart';
import '../model/mail_groups.dart';
import '../model/workspace.dart';
import 'controls.dart';
import 'icons.dart';
import 'theme.dart';

/// Selection-mode toolbar: counts, Select all/Clear/Done and the group
/// actions. Every gesture has a visible, labelled control here.
class SelectionBar extends StatelessWidget {
  const SelectionBar({super.key, required this.workspace});
  final Workspace workspace;

  Future<void> start(BuildContext context, GroupAction action) async {
    final selection = workspace.selection;
    final groups = workspace.groups;
    if (selection == null || groups == null) return;
    String? folder;
    if (action == GroupAction.move) {
      folder = await showDialog<String>(
        context: context,
        builder: (context) => SimpleDialog(
          title: const Text('Move selected messages'),
          children: workspace.folders
              .where((f) => f != 'Drafts')
              .map(
                (folder) => SimpleDialogOption(
                  onPressed: () => Navigator.pop(context, folder),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(vertical: 9),
                    child: Text(folder),
                  ),
                ),
              )
              .toList(),
        ),
      );
      if (folder == null || !context.mounted) return;
    }
    final review = await groups.prepare(selection, action, folder: folder);
    if (review == null || !context.mounted) return;
    await showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (_) => GroupReviewDialog(workspace: workspace),
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    final selection = workspace.selection;
    final groups = workspace.groups;
    if (selection == null || !selection.mode) return const SizedBox.shrink();
    final count = selection.count;
    final busy = selection.pending || (groups?.preparing ?? false);
    final actionable = selection.ready && !(groups?.preparing ?? false);
    final status = selection.error != null
        ? selection.error!
        : selection.warning != null
        ? selection.warning!
        : busy
        ? 'Updating selection…'
        : count == 0
        ? 'No messages selected'
        : count == workspace.resultCount
        ? 'All $count selected'
        : '$count selected';
    return Material(
      color: c.tint,
      child: Semantics(
        container: true,
        label: 'Selection toolbar',
        child: Padding(
          padding: const EdgeInsets.fromLTRB(12, 4, 4, 4),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  Expanded(
                    child: Semantics(
                      liveRegion: true,
                      child: Text(
                        status,
                        style: TextStyle(
                          fontSize: ShepText.body,
                          fontWeight: FontWeight.w600,
                          color: selection.error != null ? c.flag : c.accent,
                        ),
                      ),
                    ),
                  ),
                  if (selection.error != null)
                    TextButton(
                      onPressed: selection.retry,
                      child: const Text('Retry'),
                    ),
                  TextButton(
                    onPressed: busy ? null : selection.all,
                    child: const Text('Select all'),
                  ),
                  TextButton(
                    onPressed: busy || count == 0 ? null : selection.clear,
                    child: const Text('Clear'),
                  ),
                  TextButton(
                    onPressed: selection.done,
                    child: const Text('Done'),
                  ),
                ],
              ),
              Wrap(
                children: [
                  for (final action in GroupAction.values)
                    IconButton(
                      tooltip: '${action.label} selected',
                      onPressed: actionable && count > 0
                          ? () => unawaited(start(context, action))
                          : null,
                      icon: ShepIcon(
                        action.icon,
                        size: 18,
                        color: actionable && count > 0 ? c.accent : c.muted,
                      ),
                    ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Frozen review: exact counts per account and folder before approval.
class GroupReviewDialog extends StatelessWidget {
  const GroupReviewDialog({super.key, required this.workspace});
  final Workspace workspace;

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    return ListenableBuilder(
      listenable: workspace,
      builder: (context, _) {
        final groups = workspace.groups;
        final review = groups?.review;
        if (groups == null || review == null) {
          return const SizedBox.shrink();
        }
        final skipped = review.count('skipped');
        String accountLabel(String id) =>
            workspace.accountRepository?.mailAccounts
                .where((a) => a.id == id)
                .firstOrNull
                ?.email ??
            id;
        return AlertDialog(
          title: Text(review.title),
          content: SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'This applies to the frozen selection, including messages on other pages. Messages changed since the review are skipped.',
                  style: TextStyle(
                    fontSize: ShepText.secondary,
                    color: c.muted,
                  ),
                ),
                const SizedBox(height: 12),
                for (final group in review.groups)
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: 3),
                    child: Text(
                      '${accountLabel(group['account'] as String)} · ${group['folder'] == 'INBOX' ? 'Inbox' : group['folder']}: ${group['total']} (${group['unread']} unread, ${group['starred']} flagged)',
                      style: TextStyle(fontSize: ShepText.body, color: c.text),
                    ),
                  ),
                if (skipped > 0)
                  Padding(
                    padding: const EdgeInsets.only(top: 8),
                    child: Text(
                      '$skipped no longer cached and will be skipped.',
                      style: TextStyle(
                        fontSize: ShepText.secondary,
                        color: c.muted,
                      ),
                    ),
                  ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () {
                unawaited(groups.decline());
                Navigator.pop(context);
              },
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: review.total == 0
                  ? null
                  : () {
                      unawaited(groups.approve());
                      Navigator.pop(context);
                    },
              child: Text(review.verb),
            ),
          ],
        );
      },
    );
  }
}

/// Progress, completion and attention notices for group actions.
class GroupActionBanner extends StatelessWidget {
  const GroupActionBanner({super.key, required this.workspace});
  final Workspace workspace;

  void openHistory(BuildContext context) {
    Navigator.push(
      context,
      MaterialPageRoute<void>(
        builder: (_) => GroupHistoryScreen(workspace: workspace),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    final groups = workspace.groups;
    if (groups == null) return const SizedBox.shrink();
    final active = groups.active;
    final paused = groups.jobs.where((j) => j.paused).firstOrNull;
    final completed = groups.completed;
    final attention = groups.needingReview.fold(0, (n, j) => n + j.attention);
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (groups.error case final error?)
          Semantics(
            liveRegion: true,
            child: NoticeBar(
              error: true,
              trailing: [
                TextButton(
                  onPressed: () => unawaited(groups.pump()),
                  child: const Text('Retry'),
                ),
                IconButton(
                  tooltip: 'Dismiss group error',
                  onPressed: () {
                    groups.error = null;
                    groups.changed();
                  },
                  icon: const ShepIcon('close', size: 18),
                ),
              ],
              child: Text(error),
            ),
          ),
        if (active != null)
          Semantics(
            liveRegion: true,
            child: NoticeBar(
              trailing: [
                TextButton(
                  onPressed: () => unawaited(groups.pause(active)),
                  child: const Text('Pause'),
                ),
                if (active.canUndo)
                  TextButton(
                    onPressed: () => unawaited(groups.undo(active)),
                    child: const Text('Undo'),
                  ),
                IconButton(
                  tooltip: 'Open group History',
                  onPressed: () => openHistory(context),
                  icon: const ShepIcon('clock', size: 18),
                ),
              ],
              child: Text(active.status),
            ),
          )
        else if (paused != null)
          Semantics(
            liveRegion: true,
            child: NoticeBar(
              trailing: [
                TextButton(
                  onPressed: () => unawaited(groups.resume(paused)),
                  child: const Text('Resume'),
                ),
                TextButton(
                  onPressed: () => openHistory(context),
                  child: const Text('History'),
                ),
              ],
              child: Text('${paused.title}: ${paused.status}'),
            ),
          )
        else if (attention > 0)
          Semantics(
            liveRegion: true,
            child: Material(
              color: c.errorSurface,
              child: Padding(
                padding: const EdgeInsets.only(left: 16, right: 4),
                child: Row(
                  children: [
                    Expanded(
                      child: Text(
                        '$attention group action ${attention == 1 ? 'step needs' : 'steps need'} review.',
                        style: TextStyle(color: c.text),
                      ),
                    ),
                    TextButton(
                      onPressed: () => openHistory(context),
                      child: const Text('History'),
                    ),
                  ],
                ),
              ),
            ),
          ),
        if (completed != null)
          Semantics(
            liveRegion: true,
            child: ToastCard(
              trailing: [
                if (completed.canUndo)
                  TextButton(
                    onPressed: () => unawaited(groups.undo(completed)),
                    child: const Text('Undo'),
                  ),
                IconButton(
                  tooltip: 'Dismiss group notification',
                  onPressed: groups.dismiss,
                  icon: const ShepIcon('close', size: 18),
                ),
              ],
              child: Text(completed.status),
            ),
          ),
      ],
    );
  }
}

/// At most 20 groups with per-state counts; each group pages 50 items.
class GroupHistoryScreen extends StatefulWidget {
  const GroupHistoryScreen({super.key, required this.workspace});
  final Workspace workspace;
  @override
  State<GroupHistoryScreen> createState() => _GroupHistoryScreenState();
}

class _GroupHistoryScreenState extends State<GroupHistoryScreen> {
  String? expanded;
  List<GroupItem> rows = [];
  int? nextAfter;
  bool loading = false;
  String? itemsError;
  int _request = 0;
  MailGroups get groups => widget.workspace.groups!;

  @override
  void initState() {
    super.initState();
    unawaited(groups.refreshHistory());
  }

  Future<void> load(GroupJob job, {bool more = false}) async {
    final request = ++_request;
    setState(() {
      loading = true;
      itemsError = null;
      if (!more) {
        rows = [];
        nextAfter = null;
      }
    });
    try {
      final page = await groups.items(job, after: more ? nextAfter : null);
      if (request != _request || !mounted) return;
      setState(() {
        rows = more ? [...rows, ...page.rows] : page.rows;
        nextAfter = page.nextAfter;
      });
    } catch (e) {
      if (request != _request || !mounted) return;
      setState(() => itemsError = 'Could not read these messages. $e');
    } finally {
      if (request == _request && mounted) setState(() => loading = false);
    }
  }

  void toggle(GroupJob job) {
    if (expanded == job.id) {
      setState(() => expanded = null);
      return;
    }
    setState(() => expanded = job.id);
    unawaited(load(job));
  }

  Widget item(GroupJob job, GroupItem item) {
    final c = ShepColors.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 4, 8, 4),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  item.subject,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: ShepText.body, color: c.text),
                ),
                Text(
                  '${item.label}${item.reason != null ? ' · ${item.reason}' : ''}',
                  style: TextStyle(
                    fontSize: ShepText.secondary,
                    color: item.needsAttention ? c.flag : c.muted,
                  ),
                ),
              ],
            ),
          ),
          if (item.canRetry)
            TextButton(
              onPressed: () async {
                await groups.retry(job, item);
                if (mounted) unawaited(load(job));
              },
              child: const Text('Retry'),
            ),
          if (item.canAccept)
            TextButton(
              onPressed: () async {
                await groups.accept(job, item);
                if (mounted) unawaited(load(job));
              },
              child: const Text('Accept current state'),
            ),
        ],
      ),
    );
  }

  Widget card(GroupJob job) {
    final c = ShepColors.of(context);
    final open = expanded == job.id;
    return Card(
      semanticContainer: false,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          InkWell(
            onTap: () => toggle(job),
            borderRadius: BorderRadius.circular(ShepRadius.card),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(16, 14, 8, 6),
              child: Row(
                children: [
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          job.title,
                          style: TextStyle(
                            fontSize: ShepText.cardTitle,
                            fontWeight: FontWeight.w600,
                            color: c.text,
                          ),
                        ),
                        const SizedBox(height: 4),
                        Text(
                          job.status,
                          style: TextStyle(
                            fontSize: ShepText.secondary,
                            color: job.attention > 0 ? c.flag : c.muted,
                          ),
                        ),
                        if (job.error case final error?)
                          Text(
                            error,
                            style: TextStyle(
                              fontSize: ShepText.secondary,
                              color: c.flag,
                            ),
                          ),
                      ],
                    ),
                  ),
                  Semantics(
                    label: '${open ? 'Hide' : 'Show'} messages of ${job.title}',
                    button: true,
                    child: ShepIcon(
                      open ? 'up' : 'chevron-down',
                      size: 18,
                      color: c.muted,
                    ),
                  ),
                ],
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 0, 8, 6),
            child: Wrap(
              alignment: WrapAlignment.end,
              children: [
                if (job.canPause)
                  TextButton(
                    onPressed: () => unawaited(groups.pause(job)),
                    child: const Text('Pause'),
                  ),
                if (job.paused)
                  TextButton(
                    onPressed: () => unawaited(groups.resume(job)),
                    child: const Text('Resume'),
                  ),
                if (job.canUndo)
                  TextButton(
                    onPressed: () => unawaited(groups.undo(job)),
                    child: const Text('Undo'),
                  ),
                if (job.canRemove)
                  TextButton(
                    onPressed: () => unawaited(groups.remove(job)),
                    child: const Text('Remove'),
                  ),
              ],
            ),
          ),
          if (open) ...[
            Divider(color: c.border, height: 1),
            if (itemsError case final error?)
              Padding(
                padding: const EdgeInsets.all(16),
                child: Text(error, style: TextStyle(color: c.flag)),
              ),
            for (final row in rows) item(job, row),
            if (loading) const LinearProgressIndicator(),
            if (nextAfter != null && !loading)
              TextButton(
                onPressed: () => unawaited(load(job, more: true)),
                child: const Text('Load next 50 messages'),
              ),
            const SizedBox(height: 6),
          ],
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.workspace,
    builder: (context, _) {
      final c = ShepColors.of(context);
      final jobs = groups.jobs;
      return Scaffold(
        appBar: AppBar(
          title: const Text('Group History'),
          actions: [
            IconButton(
              tooltip: 'Refresh History',
              onPressed: () => unawaited(groups.refreshHistory()),
              icon: const ShepIcon('sync', size: 20),
            ),
          ],
        ),
        body: Column(
          children: [
            if (groups.error case final error?)
              NoticeBar(
                error: true,
                trailing: [
                  IconButton(
                    tooltip: 'Dismiss group error',
                    onPressed: () {
                      groups.error = null;
                      groups.changed();
                    },
                    icon: const ShepIcon('close', size: 18),
                  ),
                ],
                child: Text(error),
              ),
            if (groups.historyError case final error?)
              NoticeBar(
                error: true,
                trailing: [
                  TextButton(
                    onPressed: () => unawaited(groups.refreshHistory()),
                    child: const Text('Retry'),
                  ),
                ],
                child: Text(error),
              ),
            Expanded(
              child: jobs.isEmpty
                  ? const EmptyState(
                      icon: 'clock',
                      title: 'No group actions yet',
                      message:
                          'Select messages, choose an action and approve the review to see it here. History keeps the latest 20.',
                    )
                  : ListView(
                      padding: const EdgeInsets.all(12),
                      children: [for (final job in jobs) card(job)],
                    ),
            ),
            if (groups.historyLoading) LinearProgressIndicator(color: c.accent),
          ],
        ),
      );
    },
  );
}
