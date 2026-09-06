import 'package:flutter/material.dart';
import '../data/outgoing.dart';
import '../model/workspace.dart';
import 'composer.dart';

class OutboxScreen extends StatefulWidget {
  const OutboxScreen({super.key, required this.workspace});
  final Workspace workspace;
  @override
  State<OutboxScreen> createState() => _OutboxScreenState();
}

class _OutboxScreenState extends State<OutboxScreen> {
  OutgoingPage? page;
  String? error, notice, recoveredDraft;
  bool loading = true, busy = false;
  int generation = 0;
  final reviewed = <String>{};
  final copyReviewed = <String>{};
  OutgoingRepository get repository =>
      widget.workspace.repository as OutgoingRepository;
  @override
  void initState() {
    super.initState();
    load();
  }

  Future<void> load([int? offset]) async {
    final current = ++generation;
    setState(() => loading = true);
    try {
      final result = await repository.outbox(
        offset: offset ?? page?.offset ?? 0,
      );
      if (!mounted || current != generation) return;
      setState(() {
        page = result;
        loading = false;
        error = null;
        reviewed.clear();
        copyReviewed.clear();
      });
    } catch (e) {
      if (mounted && current == generation) {
        setState(() {
          loading = false;
          error = '$e';
        });
      }
    }
  }

  Future<void> recover(OutgoingEntry entry, OutgoingAction action) async {
    if (busy || loading) return;
    setState(() {
      busy = true;
      error = null;
      notice = null;
      recoveredDraft = null;
    });
    try {
      final result = await widget.workspace.recoverOutgoing(
        entry,
        action,
        confirmed: action == OutgoingAction.copySent
            ? copyReviewed.contains(entry.id)
            : reviewed.contains(entry.id),
      );
      if (!mounted) return;
      setState(() {
        notice = widget.workspace.notice;
        recoveredDraft = result.draftId;
      });
      await load();
    } catch (e) {
      if (mounted) {
        await load();
        if (mounted) setState(() => error = '$e');
      }
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Widget card(OutgoingEntry entry) {
    final enabled = !loading && !busy;
    return Card(
      key: ValueKey('outgoing-${entry.id}'),
      margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              entry.subject.isEmpty ? 'Untitled message' : entry.subject,
              style: Theme.of(context).textTheme.titleMedium,
            ),
            const SizedBox(height: 8),
            Text(entry.label),
            if (entry.from.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text('From: ${entry.from}'),
              ),
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text('To: ${entry.to}'),
            ),
            if (entry.uncertain) ...[
              const Padding(
                padding: EdgeInsets.only(top: 12),
                child: Text(
                  'This message may already have been sent. Check your provider’s Sent folder or the recipient before making a decision.',
                ),
              ),
              CheckboxListTile(
                contentPadding: EdgeInsets.zero,
                controlAffinity: ListTileControlAffinity.leading,
                value: reviewed.contains(entry.id),
                onChanged: enabled
                    ? (value) => setState(() {
                        if (value == true) {
                          reviewed.add(entry.id);
                        } else {
                          reviewed.remove(entry.id);
                        }
                      })
                    : null,
                title: const Text(
                  'I reviewed delivery; another send could create a duplicate',
                ),
              ),
            ],
            if (entry.sentError != null && entry.sentError != error)
              Padding(
                padding: const EdgeInsets.only(top: 12),
                child: Text(
                  entry.sentError!,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              ),
            if (entry.copyUncertain && entry.canCopySent)
              CheckboxListTile(
                contentPadding: EdgeInsets.zero,
                controlAffinity: ListTileControlAffinity.leading,
                value: copyReviewed.contains(entry.id),
                onChanged: enabled
                    ? (value) => setState(() {
                        if (value == true) {
                          copyReviewed.add(entry.id);
                        } else {
                          copyReviewed.remove(entry.id);
                        }
                      })
                    : null,
                title: const Text(
                  'I checked the server folder; uploading another Sent copy could create a duplicate',
                ),
              ),
            const SizedBox(height: 8),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                if (!entry.delivered || !entry.canCheckSent)
                  OutlinedButton(
                    onPressed: enabled
                        ? () => recover(entry, OutgoingAction.check)
                        : null,
                    child: const Text('Check delivery status'),
                  ),
                if (entry.canCheckSent)
                  OutlinedButton(
                    onPressed: enabled
                        ? () => recover(entry, OutgoingAction.checkSent)
                        : null,
                    child: Text(
                      entry.sent == 'saved'
                          ? 'Finish saving Sent copy'
                          : 'Check provider Sent',
                    ),
                  ),
                if (entry.canCopySent && entry.sent != 'saved')
                  OutlinedButton(
                    onPressed:
                        enabled &&
                            (!entry.copyUncertain ||
                                copyReviewed.contains(entry.id))
                        ? () => recover(entry, OutgoingAction.copySent)
                        : null,
                    child: const Text('Save server Sent copy'),
                  ),
                if (entry.delivered)
                  OutlinedButton(
                    onPressed: enabled
                        ? () => recover(entry, OutgoingAction.local)
                        : null,
                    child: const Text('Keep local copy'),
                  )
                else if (!entry.active) ...[
                  OutlinedButton(
                    onPressed:
                        enabled &&
                            (!entry.uncertain || reviewed.contains(entry.id))
                        ? () => recover(entry, OutgoingAction.backToDrafts)
                        : null,
                    child: const Text('Return to drafts'),
                  ),
                  if (entry.uncertain)
                    OutlinedButton(
                      onPressed: enabled && reviewed.contains(entry.id)
                          ? () => recover(entry, OutgoingAction.mark)
                          : null,
                      child: const Text('Record as sent'),
                    ),
                ],
              ],
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(
      title: const Text('Outbox'),
      actions: [
        IconButton(
          tooltip: 'Refresh Outbox',
          onPressed: busy || loading ? null : () => load(),
          icon: const Icon(Icons.refresh),
        ),
      ],
    ),
    body: SafeArea(
      child: Column(
        children: [
          if (loading || busy)
            const LinearProgressIndicator(semanticsLabel: 'Updating Outbox'),
          if (error != null || notice != null)
            Padding(
              padding: const EdgeInsets.all(16),
              child: Semantics(
                liveRegion: true,
                child: Column(
                  children: [
                    Text(
                      error ?? notice!,
                      style: TextStyle(
                        color: error == null
                            ? null
                            : Theme.of(context).colorScheme.error,
                      ),
                    ),
                    if (recoveredDraft != null &&
                        widget.workspace.drafts.containsKey(recoveredDraft))
                      TextButton(
                        onPressed: () => Navigator.push(
                          context,
                          MaterialPageRoute<void>(
                            builder: (_) => Composer(
                              workspace: widget.workspace,
                              draft: widget.workspace.drafts[recoveredDraft]!,
                            ),
                          ),
                        ),
                        child: const Text('Open recovered draft'),
                      ),
                  ],
                ),
              ),
            ),
          Expanded(
            child: page == null
                ? Center(
                    child: loading
                        ? const Text('Loading Outbox…')
                        : TextButton(
                            onPressed: () => load(),
                            child: const Text('Retry Outbox'),
                          ),
                  )
                : page!.total == 0
                ? const Center(
                    child: Text('No outgoing messages need attention.'),
                  )
                : ListView.builder(
                    itemCount: page!.rows.length,
                    itemBuilder: (_, index) => card(page!.rows[index]),
                  ),
          ),
          if (page != null && page!.total > 0)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  IconButton(
                    tooltip: 'Previous Outbox page',
                    onPressed: !busy && !loading && page!.offset > 0
                        ? () => load(page!.offset - 20)
                        : null,
                    icon: const Icon(Icons.chevron_left),
                  ),
                  Text(
                    '${page!.offset + 1}–${page!.offset + page!.rows.length} of ${page!.total}',
                  ),
                  IconButton(
                    tooltip: 'Next Outbox page',
                    onPressed:
                        !busy &&
                            !loading &&
                            page!.offset + page!.rows.length < page!.total
                        ? () => load(page!.offset + 20)
                        : null,
                    icon: const Icon(Icons.chevron_right),
                  ),
                ],
              ),
            ),
        ],
      ),
    ),
  );
}
