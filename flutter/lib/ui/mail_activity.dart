import 'dart:async';

import 'package:flutter/material.dart';

import '../data/repository.dart';
import '../model/workspace.dart';
import 'theme.dart';

class MailActivityScreen extends StatefulWidget {
  const MailActivityScreen({super.key, required this.workspace});

  final Workspace workspace;

  @override
  State<MailActivityScreen> createState() => _MailActivityScreenState();
}

class _MailActivityScreenState extends State<MailActivityScreen> {
  static const pageSize = 50;
  List<MailActivity> rows = const [];
  bool loading = false;
  bool hasMore = false;
  int offset = 0;
  String? error;
  int request = 0;

  MailActivityRepository get repository =>
      widget.workspace.repository as MailActivityRepository;

  @override
  void initState() {
    super.initState();
    unawaited(load());
  }

  Future<void> load({bool more = false}) async {
    final current = ++request;
    setState(() {
      loading = true;
      error = null;
      if (!more) rows = const [];
    });
    try {
      final nextOffset = more ? offset + rows.length : 0;
      final page = await repository.mailActions(offset: nextOffset);
      if (!mounted || current != request) return;
      setState(() {
        rows = page;
        offset = nextOffset;
        hasMore = page.length == pageSize;
        error = widget.workspace.mailResumeQueryError;
      });
    } catch (e) {
      if (!mounted || current != request) return;
      setState(() => error = 'Could not load mail Activity. $e');
    } finally {
      if (mounted && current == request) setState(() => loading = false);
    }
  }

  Future<void> resumeSaved() async {
    await widget.workspace.refreshMailActivity(resume: true);
    if (!mounted) return;
    await load();
  }

  Future<void> cancel(MailActivity action) async {
    await widget.workspace.cancelMailActivity(action);
    if (mounted) {
      await load();
      if (mounted) setState(() => error = widget.workspace.error);
    }
  }

  Future<void> undo(MailActivity action) async {
    await widget.workspace.undoMailActivity(
      action,
      onAdmitted: () {
        if (mounted) unawaited(load());
      },
    );
    if (mounted) {
      await load();
      if (mounted) setState(() => error = widget.workspace.error);
    }
  }

  Future<void> review(MailActivity action) async {
    await widget.workspace.retryMailActivity(action);
    if (!mounted) return;
    await load();
    if (mounted) setState(() => error = widget.workspace.error);
  }

  Widget row(MailActivity action) {
    final colours = ShepColors.of(context);
    return ListTile(
      title: Text(action.error ?? _description(action)),
      subtitle: Text(_status(action.status)),
      trailing: switch (action.status) {
        'queued' || 'waiting' => TextButton(
          onPressed: () => unawaited(cancel(action)),
          child: const Text('Cancel'),
        ),
        'rejected' => TextButton(
          onPressed: () => unawaited(review(action)),
          child: const Text('Retry'),
        ),
        'repair' || 'uncertain' => TextButton(
          onPressed: () => unawaited(review(action)),
          child: const Text('Review'),
        ),
        'succeeded' => TextButton(
          onPressed: () => unawaited(undo(action)),
          child: const Text('Undo'),
        ),
        _ => null,
      },
      textColor: action.needsReview ? colours.flag : colours.text,
    );
  }

  String _description(MailActivity action) {
    if (action.fields['folder'] case final String folder) {
      return 'Move mail to $folder';
    }
    if (action.fields['unread'] case final bool unread) {
      return unread ? 'Mark mail unread' : 'Mark mail read';
    }
    if (action.fields['starred'] case final bool starred) {
      return starred ? 'Flag mail' : 'Remove flag';
    }
    return 'Mail change';
  }

  String _status(String status) => switch (status) {
    'queued' => 'Queued',
    'waiting' => 'Waiting',
    'running' => 'Sending',
    'succeeded' => 'Synced',
    'rejected' => 'Could not sync',
    'uncertain' || 'repair' => 'Needs checking',
    'cancelled' => 'Cancelled',
    _ => status,
  };

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(
      title: const Text('Mail Activity'),
      actions: [
        TextButton(
          onPressed: loading ? null : load,
          child: const Text('Refresh'),
        ),
      ],
    ),
    body: rows.isEmpty && !loading && error == null
        ? const Center(child: Text('No recent mail activity.'))
        : ListView(
            children: [
              if (error != null)
                ListTile(
                  title: Text(error!),
                  trailing: TextButton(
                    onPressed: error == widget.workspace.mailResumeQueryError
                        ? resumeSaved
                        : load,
                    child: const Text('Retry'),
                  ),
                ),
              for (final action in rows) row(action),
              if (loading)
                const Center(
                  child: Padding(
                    padding: EdgeInsets.all(16),
                    child: CircularProgressIndicator(),
                  ),
                ),
              if (hasMore && !loading)
                Center(
                  child: TextButton(
                    onPressed: () => unawaited(load(more: true)),
                    child: const Text('Load more'),
                  ),
                ),
            ],
          ),
  );
}
