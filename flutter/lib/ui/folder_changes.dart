import 'dart:async';
import 'package:flutter/material.dart';
import '../data/folders.dart';
import '../model/folder_creations.dart';
import 'icons.dart';

Future<void> showFolderChange(
  BuildContext context,
  FolderCreations controller,
) => showDialog<void>(
  context: context,
  builder: (_) => FolderChangeDialog(controller: controller),
);

class FolderChangeDialog extends StatefulWidget {
  const FolderChangeDialog({super.key, required this.controller});
  final FolderCreations controller;
  @override
  State<FolderChangeDialog> createState() => _FolderChangeDialogState();
}

class _FolderChangeDialogState extends State<FolderChangeDialog> {
  final name = TextEditingController();
  List<FolderAccount> options = const [];
  FolderAccount? account;
  String? source, parent, error;
  String action = 'Rename', id = FolderCreations.identity();
  Map<String, dynamic>? review;
  bool busy = false;
  @override
  void initState() {
    super.initState();
    options = List.of(widget.controller.accounts);
    account = options.firstOrNull;
    if (options.isEmpty) unawaited(load());
  }

  Future<void> load() async {
    await widget.controller.refresh();
    if (!mounted) return;
    setState(() {
      options = List.of(widget.controller.accounts);
      account = options.firstOrNull;
    });
  }

  @override
  void dispose() {
    name.dispose();
    super.dispose();
  }

  Future<void> prepare() async {
    if (busy || account == null || source == null) return;
    setState(() {
      busy = true;
      error = null;
    });
    try {
      final result = await widget.controller.reviewChange(
        account!,
        source!,
        switch (action) {
          'Rename' => {
            'Rename': {'name': name.text},
          },
          'Move' => {
            'Move': {'parent': parent},
          },
          _ => 'Delete',
        },
      );
      if (mounted) {
        setState(() {
          review = result;
          id = FolderCreations.identity();
        });
      }
    } catch (cause) {
      if (mounted) {
        setState(() {
          error =
              'Could not review this change. Refresh folders and finish pending account work before trying again.';
        });
      }
    } finally {
      if (mounted) {
        setState(() {
          busy = false;
        });
      }
    }
  }

  Future<void> confirm() async {
    final frozen = review;
    if (busy || frozen == null) return;
    setState(() {
      busy = true;
      error = null;
    });
    try {
      await widget.controller.admitChange(id, frozen);
      if (mounted) Navigator.pop(context);
    } catch (_) {
      if (mounted) {
        setState(() {
          busy = false;
          error =
              'Could not save this exact review. Retry keeps its request identity; go back to review changed folders.';
        });
      }
    }
  }

  Widget choice(
    String label,
    String? value,
    List<DropdownMenuItem<String>> items,
    ValueChanged<String?> changed,
  ) => DropdownButtonFormField<String>(
    initialValue: value,
    isExpanded: true,
    decoration: InputDecoration(labelText: label),
    icon: const ShepIcon('chevron-down', size: 18),
    items: items,
    onChanged: busy ? null : changed,
  );

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(
      review == null
          ? 'Manage folders'
          : 'Review folder ${action.toLowerCase()}',
    ),
    content: SizedBox(
      width: 360,
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (review == null) ...[
              choice(
                'Account',
                account?.id,
                [
                  for (final entry in options)
                    DropdownMenuItem(
                      value: entry.id,
                      child: Text(
                        '${entry.label} (${entry.email})',
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                ],
                (value) => setState(() {
                  account = options
                      .where((entry) => entry.id == value)
                      .firstOrNull;
                  source = null;
                  parent = null;
                }),
              ),
              const SizedBox(height: 12),
              KeyedSubtree(
                key: ValueKey(account?.id),
                child: choice('Folder', source, [
                  for (final value
                      in ((account?.data['change_names'] ??
                                      account?.data['names'])
                                  as List? ??
                              const [])
                          .cast<String>()
                          .where((name) => name.toUpperCase() != 'INBOX'))
                    DropdownMenuItem(
                      value: value,
                      child: Text(
                        account!.parentLabel(value),
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                ], (value) => setState(() => source = value)),
              ),
              const SizedBox(height: 12),
              choice('Action', action, [
                for (final value in ['Rename', 'Move', 'Delete'])
                  DropdownMenuItem(value: value, child: Text(value)),
              ], (value) => setState(() => action = value!)),
              const SizedBox(height: 12),
              if (action == 'Rename')
                TextField(
                  controller: name,
                  enabled: !busy,
                  decoration: const InputDecoration(labelText: 'New name'),
                ),
              if (action == 'Move')
                choice(
                  'Destination parent',
                  parent ?? '',
                  [
                    const DropdownMenuItem(
                      value: '',
                      child: Text('Account root'),
                    ),
                    for (final value in account?.parents ?? <String>[])
                      DropdownMenuItem(
                        value: value,
                        child: Text(
                          account!.parentLabel(value),
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                  ],
                  (value) =>
                      setState(() => parent = value == '' ? null : value),
                ),
              const SizedBox(height: 12),
              const Text(
                'Changes apply to this account and the reviewed subtree. Sent and special-use folders need a separate account migration.',
              ),
            ] else ...[
              Text(account?.label ?? 'Account'),
              const SizedBox(height: 8),
              Text(
                account?.parentLabel(source ?? '') ?? '',
                style: Theme.of(context).textTheme.titleMedium,
              ),
              Text(
                '${(review!['plan']['members'] as List).length} folders · ${review!['messages']} cached messages',
              ),
              const SizedBox(height: 12),
              if (action == 'Rename') Text('New name: ${name.text}'),
              if (action == 'Move')
                Text(
                  'Destination: ${parent == null ? 'Account root' : account!.parentLabel(parent!)}',
                ),
              Text(
                action == 'Delete'
                    ? 'This deletes the reviewed server folders and their mail. It cannot be undone.'
                    : 'This changes the reviewed server folder names. It cannot be undone here.',
              ),
              const SizedBox(height: 12),
              const Text(
                'The result appears after saving here. Server work continues in the background; Folder activity keeps any errors.',
              ),
            ],
            if (error != null)
              Padding(
                padding: const EdgeInsets.only(top: 12),
                child: Text(error!),
              ),
          ],
        ),
      ),
    ),
    actions: [
      TextButton(
        onPressed: busy ? null : () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
      if (review != null)
        TextButton(
          onPressed: busy ? null : () => setState(() => review = null),
          child: const Text('Back'),
        ),
      FilledButton(
        onPressed: busy
            ? null
            : review == null
            ? prepare
            : confirm,
        child: Text(
          busy
              ? 'Saving…'
              : review == null
              ? 'Review change'
              : 'Confirm $action',
        ),
      ),
    ],
  );
}
