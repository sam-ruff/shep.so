import 'package:flutter/material.dart';
import '../data/accounts.dart';
import '../model/workspace.dart';

class SentPreferencesScreen extends StatefulWidget {
  const SentPreferencesScreen({
    super.key,
    required this.workspace,
    required this.account,
  });
  final Workspace workspace;
  final MailAccount account;
  @override
  State<SentPreferencesScreen> createState() => _SentPreferencesState();
}

class _SentPreferencesState extends State<SentPreferencesScreen> {
  MailAccount get account =>
      widget.workspace.accountRepository?.mailAccounts
          .where((a) => a.id == widget.account.id)
          .firstOrNull ??
      widget.account;
  late String policy = account.protocol == 'Pop3'
      ? 'LocalOnly'
      : account.sentCopy;
  late final folder = TextEditingController(text: account.sentFolder);
  bool saving = false;
  String? error;
  @override
  void dispose() {
    folder.dispose();
    super.dispose();
  }

  Future<void> save() async {
    setState(() {
      saving = true;
      error = null;
    });
    try {
      await (widget.workspace.repository as SentPreferencesRepository)
          .saveSentPreferences(widget.account.id, policy, folder.text.trim());
      if (mounted) {
        Navigator.pop(context);
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          saving = false;
          error = '$e';
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: const Text('Sent copies')),
    body: ListView(
      padding: const EdgeInsets.all(20),
      children: [
        Text(
          widget.account.email,
          style: Theme.of(context).textTheme.titleMedium,
        ),
        const SizedBox(height: 20),
        if (widget.account.protocol == 'Pop3')
          const Text('POP3 keeps Sent copies on this device.')
        else ...[
          DropdownButtonFormField<String>(
            initialValue: policy,
            isExpanded: true,
            decoration: const InputDecoration(labelText: 'Sent-copy policy'),
            items: const [
              DropdownMenuItem(
                value: 'Automatic',
                child: Text('Save a copy on the mail server'),
              ),
              DropdownMenuItem(
                value: 'ServerManaged',
                child: Text('My server saves Sent automatically'),
              ),
              DropdownMenuItem(
                value: 'LocalOnly',
                child: Text('Keep Sent on this device'),
              ),
            ],
            onChanged: saving ? null : (v) => setState(() => policy = v!),
          ),
          const SizedBox(height: 16),
          if (policy != 'LocalOnly')
            TextField(
              controller: folder,
              enabled: !saving,
              decoration: const InputDecoration(
                labelText: 'Sent folder (optional)',
                helperText: 'Leave empty to discover the server’s Sent folder.',
                helperMaxLines: 2,
              ),
            ),
          const SizedBox(height: 20),
          const Text(
            'A local copy is kept after delivery. If a server upload is not acknowledged, Outbox asks you to review it before uploading another copy.',
          ),
        ],
        if (error != null)
          Padding(
            padding: const EdgeInsets.only(top: 20),
            child: Semantics(
              liveRegion: true,
              child: Text(
                error!,
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            ),
          ),
      ],
    ),
    bottomNavigationBar: SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: FilledButton(
          onPressed: saving ? null : save,
          child: Text(saving ? 'Saving…' : 'Save Sent preferences'),
        ),
      ),
    ),
  );
}
