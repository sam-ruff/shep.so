import 'dart:async';
import 'package:flutter/material.dart';
import 'package:file_selector/file_selector.dart';
import '../data/drafts.dart';
import '../data/outgoing.dart';
import 'outbox.dart';
import '../model/mail.dart';
import '../model/workspace.dart';
import 'controls.dart';
import 'icons.dart';

class Composer extends StatefulWidget {
  const Composer({super.key, required this.workspace, required this.draft});
  final Workspace workspace;
  final Draft draft;
  @override
  State<Composer> createState() => _ComposerState();
}

class _ComposerState extends State<Composer> {
  late final to = TextEditingController(text: widget.draft.to);
  late final cc = TextEditingController(text: widget.draft.cc);
  late final bcc = TextEditingController(text: widget.draft.bcc);
  late final subject = TextEditingController(text: widget.draft.subject);
  late final body = TextEditingController(text: widget.draft.body);
  late String accountId = widget.draft.accountId.isNotEmpty
      ? widget.draft.accountId
      : widget.workspace.accountRepository?.mailAccounts.firstOrNull?.id ?? '';
  late int revision = widget.draft.revision;
  late DraftFiles fileState = DraftFiles(
    widget.draft.attachments,
    widget.draft.fileRevision,
  );
  DraftRepository? get filesRepository =>
      widget.workspace.repository is DraftRepository
      ? widget.workspace.repository as DraftRepository
      : null;
  bool busy = false, saving = false, showCopy = false, checking = false;
  String? error, delivery;
  Timer? autosave;
  Future<void> writes = Future.value();
  bool get locked => busy || checking || delivery != null;
  Draft get draft => Draft(
    id: widget.draft.id,
    accountId: accountId,
    to: to.text,
    cc: cc.text,
    bcc: bcc.text,
    subject: subject.text,
    body: body.text,
    revision: revision,
    inReplyTo: widget.draft.inReplyTo,
    references: widget.draft.references,
    forward: widget.draft.forward,
    attachments: fileState.attachments,
    fileRevision: fileState.revision,
  );

  @override
  void initState() {
    super.initState();
    showCopy = cc.text.isNotEmpty || bcc.text.isNotEmpty;
    for (final c in [to, cc, bcc, subject, body]) {
      c.addListener(edited);
    }
    checkDelivery();
  }

  void edited() {
    if (locked) return;
    revision++;
    autosave?.cancel();
    autosave = Timer(const Duration(milliseconds: 500), () {
      unawaited(saveSnapshot(draft));
    });
  }

  Future<void> checkDelivery() async {
    final native = widget.workspace.accountRepository;
    if (native == null) return;
    setState(() => checking = true);
    try {
      final files = await filesRepository?.files(widget.draft.id);
      final status = await native.delivery(widget.draft.id);
      if (mounted) {
        setState(() {
          delivery = status;
          if (files != null) fileState = files;
          checking = false;
        });
      }
    } catch (e) {
      // Fail closed until the durable outgoing journal can be checked.
      if (mounted) {
        setState(() {
          error = '$e';
          checking = false;
          delivery = 'unknown';
        });
      }
    }
  }

  Future<void> saveSnapshot(Draft snapshot) async {
    if (mounted) setState(() => saving = true);
    writes = writes.then((_) async {
      final ok = await widget.workspace.saveDraft(snapshot);
      if (mounted && snapshot.revision == revision) {
        setState(() {
          saving = false;
          error = ok ? null : widget.workspace.error;
        });
      }
    });
    await writes;
  }

  Future<void> finish(bool send) async {
    if (delivery != null) {
      if (send) {
        await checkDelivery();
      } else {
        Navigator.pop(context);
      }
      return;
    }
    if (send &&
        ((to.text.trim().isEmpty &&
                cc.text.trim().isEmpty &&
                bcc.text.trim().isEmpty) ||
            subject.text.trim().isEmpty)) {
      setState(() => error = 'Add a recipient and subject before sending.');
      return;
    }
    autosave?.cancel();
    setState(() {
      busy = true;
      error = null;
    });
    await writes;
    final ok = send
        ? await widget.workspace.send(draft)
        : await widget.workspace.saveDraft(draft);
    if (!mounted) return;
    if (ok) {
      Navigator.pop(context);
    } else {
      setState(() {
        busy = false;
        error = widget.workspace.error;
      });
      if (send) await checkDelivery();
    }
  }

  Future<void> attach() async {
    final repository = filesRepository;
    if (repository == null || locked) return;
    autosave?.cancel();
    setState(() => busy = true);
    try {
      await writes;
      if (!await widget.workspace.saveDraft(draft)) {
        throw StateError(
          widget.workspace.error ?? 'Save the draft before attaching files.',
        );
      }
      final selected = await openFiles();
      if (selected.isNotEmpty) {
        final result = await repository.addFiles(
          widget.draft.id,
          selected.map((f) => SelectedAttachment(f.path, f.name)).toList(),
        );
        if (mounted) {
          setState(() {
            fileState = result;
            error = null;
          });
          widget.workspace.drafts[widget.draft.id] = draft;
        }
      }
    } catch (e) {
      if (mounted) setState(() => error = '$e');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> removeFile(DraftAttachment file) async {
    final repository = filesRepository;
    if (repository == null || locked) return;
    autosave?.cancel();
    final previous = fileState;
    setState(() {
      busy = true;
      fileState = DraftFiles(
        previous.attachments.where((a) => a.id != file.id).toList(),
        previous.revision,
      );
    });
    try {
      await writes;
      if (!await widget.workspace.saveDraft(draft)) {
        throw StateError(
          widget.workspace.error ?? 'Save the draft before removing files.',
        );
      }
      final result = await repository.removeFile(widget.draft.id, file.id);
      if (mounted) {
        setState(() {
          fileState = result;
          error = null;
        });
        widget.workspace.drafts[widget.draft.id] = draft;
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          fileState = previous;
          error = '$e';
        });
      }
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> discard() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Discard this draft?'),
        content: const Text(
          'Its saved text and attachments will be removed from this device.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Keep draft'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Discard draft'),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    autosave?.cancel();
    setState(() => busy = true);
    await writes;
    final ok = await widget.workspace.discardDraft(draft);
    if (!mounted) return;
    if (ok) {
      Navigator.pop(context);
    } else {
      setState(() {
        busy = false;
        error = widget.workspace.error;
      });
    }
  }

  @override
  void dispose() {
    autosave?.cancel();
    for (final c in [to, cc, bcc, subject, body]) {
      c.dispose();
    }
    super.dispose();
  }

  Widget field(
    TextEditingController controller,
    String label, {
    bool multiline = false,
  }) => TextField(
    controller: controller,
    readOnly: locked,
    minLines: multiline ? 10 : 1,
    maxLines: multiline ? null : 1,
    keyboardType: multiline
        ? TextInputType.multiline
        : label == 'To'
        ? TextInputType.emailAddress
        : TextInputType.text,
    decoration: InputDecoration(
      labelText: label,
      hintText: switch (label) {
        'To' || 'Cc' || 'Bcc' => 'name@example.com',
        'Subject' => 'Add a subject',
        'Message' => 'Write your message…',
        _ => null,
      },
      alignLabelWithHint: multiline,
    ),
  );

  @override
  Widget build(BuildContext context) => PopScope(
    canPop: false,
    onPopInvokedWithResult: (popped, _) {
      if (!popped && !busy && !checking) finish(false);
    },
    child: Scaffold(
      appBar: AppBar(
        title: Text(widget.draft.forward != null ? 'Forward' : 'New message'),
        leading: IconButton(
          tooltip: 'Save and close',
          onPressed: busy || checking ? null : () => finish(false),
          icon: const ShepIcon('close'),
        ),
        actions: [
          if (widget.workspace.accountRepository != null)
            IconButton(
              tooltip: 'Discard draft',
              onPressed: locked ? null : discard,
              icon: const ShepIcon('trash'),
            ),
          TextButton(
            onPressed: busy || checking ? null : () => finish(false),
            child: Text(delivery == null ? 'Save draft' : 'Close'),
          ),
          IconButton(
            tooltip: delivery == null ? 'Send' : 'Check delivery status',
            onPressed: busy || checking ? null : () => finish(true),
            icon: ShepIcon(delivery == null ? 'send' : 'sync'),
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.all(18),
        children: [
          if (widget.workspace.repository.preview)
            const Padding(
              padding: EdgeInsets.only(bottom: 16),
              child: Text('Preview • sending is disabled'),
            ),
          if (error != null)
            Padding(
              padding: const EdgeInsets.only(bottom: 16),
              child: Text(
                error!,
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            ),
          if (delivery != null)
            Padding(
              padding: const EdgeInsets.only(bottom: 16),
              child: Text(switch (delivery) {
                'delivered' =>
                  'Delivery confirmed. Open Outbox to review its Sent copy.',
                'reviewed' =>
                  'This submission was reviewed. Open its recovered draft or Sent copy.',
                'rejected' =>
                  'SMTP did not accept this message. Return it to drafts from Outbox before editing.',
                _ =>
                  'Delivery: $delivery. This saved submission will not be sent again. Open Outbox to review delivery.',
              }),
            ),
          if (delivery != null &&
              widget.workspace.repository is OutgoingRepository)
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton.icon(
                icon: const ShepIcon('outbox'),
                label: const Text('Open Outbox'),
                onPressed: () async {
                  await Navigator.push(
                    context,
                    MaterialPageRoute<void>(
                      builder: (_) => OutboxScreen(workspace: widget.workspace),
                    ),
                  );
                  if (mounted) await checkDelivery();
                },
              ),
            ),
          if (widget.workspace.accountRepository case final native?) ...[
            DropdownButtonFormField<String>(
              initialValue: native.mailAccounts.any((a) => a.id == accountId)
                  ? accountId
                  : null,
              isExpanded: true,
              icon: dropdownChevron(),
              decoration: const InputDecoration(labelText: 'From'),
              items: native.mailAccounts
                  .map(
                    (a) => DropdownMenuItem(value: a.id, child: Text(a.email)),
                  )
                  .toList(),
              onChanged: locked
                  ? null
                  : (v) {
                      if (v != null) {
                        setState(() => accountId = v);
                        edited();
                      }
                    },
            ),
            const SizedBox(height: 14),
          ],
          field(to, 'To'),
          Align(
            alignment: Alignment.centerRight,
            child: TextButton(
              onPressed: () => setState(() => showCopy = !showCopy),
              child: const Text('Cc / Bcc'),
            ),
          ),
          if (showCopy) ...[
            field(cc, 'Cc'),
            const SizedBox(height: 12),
            field(bcc, 'Bcc'),
            const SizedBox(height: 12),
          ],
          field(subject, 'Subject'),
          const SizedBox(height: 16),
          if (filesRepository != null) ...[
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: fileState.attachments
                  .map(
                    (file) => InputChip(
                      label: Text('${file.name} (${file.size} bytes)'),
                      tooltip: file.name,
                      onDeleted: locked ? null : () => removeFile(file),
                      deleteButtonTooltipMessage: 'Remove ${file.name}',
                    ),
                  )
                  .toList(),
            ),
            const SizedBox(height: 8),
            TextButton.icon(
              onPressed: locked ? null : attach,
              icon: const ShepIcon('clip'),
              label: const Text('Attach files'),
            ),
          ],
          field(body, 'Message', multiline: true),
          const SizedBox(height: 18),
          if (saving) const Text('Saving draft…'),
        ],
      ),
    ),
  );
}
