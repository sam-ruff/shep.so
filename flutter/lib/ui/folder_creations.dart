import 'dart:async';
import 'package:flutter/material.dart';
import '../data/folders.dart';
import '../model/folder_creations.dart';
import 'icons.dart';
export 'folder_changes.dart' show showFolderChange;

Future<void> showNewFolder(
  BuildContext context,
  FolderCreations controller,
) async {
  await showDialog<void>(
    context: context,
    builder: (_) => NewFolderDialog(controller: controller),
  );
}

class NewFolderDialog extends StatefulWidget {
  const NewFolderDialog({super.key, required this.controller});
  final FolderCreations controller;
  @override
  State<NewFolderDialog> createState() => _NewFolderDialogState();
}

class _NewFolderDialogState extends State<NewFolderDialog> {
  final name = TextEditingController();
  FolderAccount? account;
  List<FolderAccount> options = const [];
  String? parent, error, request, payload;
  bool saving = false, loading = false;
  @override
  void initState() {
    super.initState();
    options = List.of(widget.controller.accounts);
    account = options.firstOrNull;
    if (options.isEmpty) {
      loading = true;
      unawaited(loadOptions());
    }
  }

  Future<void> loadOptions() async {
    await widget.controller.refresh();
    if (!mounted) return;
    setState(() {
      options = List.of(widget.controller.accounts);
      account = options.firstOrNull;
      loading = false;
    });
  }

  @override
  void dispose() {
    name.dispose();
    super.dispose();
  }

  Future<void> submit() async {
    final selected = account;
    if (selected == null || name.text.trim().isEmpty || saving) return;
    final current =
        '${selected.id}\u0000${selected.connection}\u0000${parent ?? ''}\u0000${name.text}';
    if (payload != current) {
      request = FolderCreations.identity();
      payload = current;
    }
    setState(() {
      saving = true;
      error = null;
    });
    try {
      await widget.controller.admit(request!, selected, parent, name.text);
      if (mounted) Navigator.pop(context);
    } catch (_) {
      if (mounted) {
        setState(() {
          saving = false;
          error =
              'Could not save this folder request. Your name has been kept. Retry or refresh Folder activity.';
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('New folder'),
    content: SizedBox(
      width: 360,
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (loading) const Text('Loading accounts…'),
            if (!loading && options.isEmpty)
              const Text(
                'Connect a mail account in Preferences before creating a folder.',
              ),
            DropdownButtonFormField<String>(
              initialValue: account?.id,
              isExpanded: true,
              icon: const ShepIcon('chevron-down', size: 18),
              decoration: const InputDecoration(labelText: 'Account'),
              items: [
                for (final option in options)
                  DropdownMenuItem(
                    value: option.id,
                    child: Text(
                      '${option.label} (${option.email})',
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
              ],
              onChanged: saving
                  ? null
                  : (id) => setState(() {
                      account = options.where((a) => a.id == id).firstOrNull;
                      parent = null;
                    }),
            ),
            const SizedBox(height: 16),
            DropdownButtonFormField<String>(
              key: ValueKey(account?.id),
              initialValue: parent ?? '',
              isExpanded: true,
              icon: const ShepIcon('chevron-down', size: 18),
              decoration: const InputDecoration(labelText: 'Parent folder'),
              items: [
                const DropdownMenuItem(value: '', child: Text('Account root')),
                for (final value in account?.parents ?? <String>[])
                  DropdownMenuItem(
                    value: value,
                    child: Text(
                      account!.parentLabel(value),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
              ],
              onChanged: saving
                  ? null
                  : (value) =>
                        setState(() => parent = value == '' ? null : value),
            ),
            const SizedBox(height: 16),
            TextField(
              controller: name,
              enabled: !saving,
              autofocus: true,
              decoration: const InputDecoration(labelText: 'Folder name'),
              onSubmitted: (_) => unawaited(submit()),
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: 12),
            const Text(
              'The folder will appear here while it connects to the server.',
            ),
            if (error ?? widget.controller.error case final String message) ...[
              const SizedBox(height: 12),
              Text(
                message,
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            ],
          ],
        ),
      ),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
      FilledButton(
        onPressed: saving || account == null || name.text.trim().isEmpty
            ? null
            : submit,
        child: Text(saving ? 'Saving request…' : 'Create folder'),
      ),
    ],
  );
}

class FolderActivityScreen extends StatelessWidget {
  const FolderActivityScreen({super.key, required this.controller});
  final FolderCreations controller;
  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) => Scaffold(
      appBar: AppBar(
        title: const Text('Folder activity'),
        actions: [
          IconButton(
            tooltip: 'Refresh folder activity',
            onPressed: () => controller.refresh(resume: true),
            icon: const ShepIcon('sync'),
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          if (controller.error case final String error) Text(error),
          if (controller.entries.isEmpty) const Text('No folder requests yet.'),
          for (final entry in controller.entries.where(
            (entry) => entry.status != 'dismissed',
          ))
            Card(
              key: ValueKey(entry.id),
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      entry.name,
                      style: Theme.of(context).textTheme.titleMedium,
                    ),
                    Text(
                      '${controller.accounts.where((account) => account.id == entry.account).firstOrNull?.label ?? 'Account'} · ${controller.parentLabel(entry)}',
                    ),
                    const SizedBox(height: 8),
                    Text(entry.label),
                    if (entry.error case final String message) Text(message),
                    if (controller.failures[entry.id] case final String message)
                      Text(message),
                    const SizedBox(height: 8),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        if (entry.canRetry)
                          TextButton(
                            key: const ValueKey('retry'),
                            onPressed: () => controller.decide(entry, 'retry'),
                            child: const Text('Retry'),
                          ),
                        if (entry.canCheck)
                          TextButton(
                            key: const ValueKey('check'),
                            onPressed: () => controller.decide(entry, 'check'),
                            child: const Text('Check server'),
                          ),
                        if (entry.canCancel)
                          TextButton(
                            key: const ValueKey('cancel'),
                            onPressed: () => controller.decide(entry, 'cancel'),
                            child: const Text('Cancel request'),
                          ),
                        if (entry.canDismiss)
                          TextButton(
                            key: const ValueKey('dismiss'),
                            onPressed: () =>
                                controller.decide(entry, 'dismiss'),
                            child: const Text('Stop tracking'),
                          ),
                      ],
                    ),
                    if (entry.canDismiss)
                      const Text(
                        'Stopping tracking does not delete any server folder.',
                      ),
                  ],
                ),
              ),
            ),
        ],
      ),
    ),
  );
}
