import 'package:flutter/material.dart';
import '../data/accounts.dart';
import '../model/workspace.dart';

class AccountRemovalScreen extends StatefulWidget {
  const AccountRemovalScreen({
    super.key,
    required this.workspace,
    required this.account,
  });
  final Workspace workspace;
  final MailAccount account;
  @override
  State<AccountRemovalScreen> createState() => _AccountRemovalState();
}

class _AccountRemovalState extends State<AccountRemovalScreen> {
  AccountRemoval? review;
  bool busy = false, discard = false;
  String? error;
  AccountRemovalRepository get repository =>
      widget.workspace.repository as AccountRemovalRepository;
  @override
  void initState() {
    super.initState();
    reload();
  }

  Future<void> reload() async {
    setState(() {
      busy = true;
      error = null;
      review = null;
      discard = false;
    });
    try {
      final next = await repository.removalPreview(widget.account.id);
      if (mounted) setState(() => review = next);
    } catch (e) {
      if (mounted) setState(() => error = '$e');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> remove() async {
    if (busy || review == null) return;
    setState(() {
      busy = true;
      error = null;
    });
    try {
      await repository.removeAccount(review!, discard);
    } catch (e) {
      if (mounted) {
        setState(() {
          busy = false;
          error = '$e';
        });
      }
      return;
    }
    // The committed removal must stay visible even if a subsequent page read
    // or keychain cleanup fails. Neither means that the account was restored.
    await widget.workspace.accountRemoved(widget.account.id);
    if (mounted) Navigator.pop(context);
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: const Text('Remove account')),
    body: ListView(
      padding: const EdgeInsets.all(20),
      children: [
        Text(
          widget.account.email,
          style: Theme.of(context).textTheme.titleLarge,
        ),
        const SizedBox(height: 16),
        const Text(
          'Remove this account and its cached mail, drafts, attached files and delivery records from this device. Mail on the server is unchanged. Local-only mail and unsent drafts cannot be recovered here after removal.',
        ),
        const SizedBox(height: 16),
        if (review case final AccountRemoval current) ...[
          Text(
            '${current.count('messages')} cached messages · ${current.count('drafts')} drafts · ${current.count('files')} draft files · ${current.count('outgoing')} delivery records',
          ),
          if (current.unfinished)
            CheckboxListTile(
              contentPadding: EdgeInsets.zero,
              title: Text(
                'Discard ${current.count('unresolved')} unfinished delivery records and ${current.count('moves')} unfinished moves',
              ),
              subtitle: const Text(
                'Removal cannot cancel or undo an operation that reached the mail server. Check Sent and the source/destination folders before removing these recovery records.',
              ),
              value: discard,
              onChanged: busy
                  ? null
                  : (v) => setState(() => discard = v ?? false),
            ),
        ],
        if (error != null)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 16),
            child: Semantics(
              liveRegion: true,
              child: Text(
                error!,
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            ),
          ),
        TextButton.icon(
          onPressed: busy ? null : reload,
          icon: const Icon(Icons.refresh),
          label: const Text('Reload removal counts'),
        ),
        if (busy) const LinearProgressIndicator(),
      ],
    ),
    bottomNavigationBar: SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Row(
          children: [
            Expanded(
              child: OutlinedButton(
                onPressed: busy ? null : () => Navigator.pop(context),
                child: const Text('Cancel'),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: FilledButton(
                onPressed:
                    busy || review == null || (review!.unfinished && !discard)
                    ? null
                    : remove,
                style: FilledButton.styleFrom(
                  backgroundColor: Theme.of(context).colorScheme.error,
                  foregroundColor: Theme.of(context).colorScheme.onError,
                ),
                child: const Text('Remove from device'),
              ),
            ),
          ],
        ),
      ),
    ),
  );
}
