import 'mail_error.dart';
import 'package:flutter/services.dart';
import '../data/message_search.dart';
import '../model/message_find.dart';
import 'message_find.dart';
import '../data/attachments.dart';
import 'package:flutter/material.dart';
import '../model/mail.dart';
import '../model/workspace.dart';
import 'composer.dart';

class Reader extends StatefulWidget {
  const Reader({
    super.key,
    required this.workspace,
    required this.id,
    required this.act,
  });
  final Workspace workspace;
  final String id;
  final void Function(String, MailAction) act;
  @override
  State<Reader> createState() => _ReaderState();
}

class _ReaderState extends State<Reader> {
  Workspace get workspace => widget.workspace;
  String get id => widget.id;
  void Function(String, MailAction) get act => widget.act;
  String? saving, fileStatus;
  bool fileError = false;
  late final MessageFind find;
  final findQuery = TextEditingController();
  final findFocus = FocusNode();
  bool? quoteExpanded;
  String? quoteMode;
  void syncFind() {
    final mail = workspace.mail(id);
    final mode = workspace.preferences.quoteMode;
    if (mode != quoteMode) {
      quoteMode = mode;
      quoteExpanded = null;
    }
    final parts = (mail?.body ?? '').split('\n>');
    final quote = parts.skip(1).join('\n>').trim();
    find.setSource(id, [
      parts.first,
      if (quote.isNotEmpty &&
          mode != 'Latest only' &&
          (quoteExpanded ?? mode == 'Expanded'))
        quote,
    ]);
  }

  void findChanged() {
    if (mounted) setState(() {});
  }

  void openFind() {
    find.show();
    if (workspace.mail(id)?.bodyLoaded == false) workspace.loadBody(id);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        findFocus.requestFocus();
        findQuery.selection = TextSelection(
          baseOffset: 0,
          extentOffset: findQuery.text.length,
        );
      }
    });
  }

  void closeFind() {
    find.close();
    findFocus.unfocus();
  }

  @override
  void initState() {
    super.initState();
    workspace.retainReader(id);
    final repository = workspace.repository;
    find = MessageFind(
      repository is TextSearchRepository
          ? (repository as TextSearchRepository).findText
          : previewFind,
    );
    find.addListener(findChanged);
    workspace.addListener(syncFind);
    syncFind();
  }

  @override
  void didUpdateWidget(Reader old) {
    super.didUpdateWidget(old);
    if (old.id != id || old.workspace != workspace) {
      old.workspace.removeListener(syncFind);
      old.workspace.releaseReader(old.id);
      workspace.retainReader(id);
      workspace.addListener(syncFind);
      quoteExpanded = null;
      syncFind();
    }
  }

  @override
  void dispose() {
    workspace.removeListener(syncFind);
    find.dispose();
    findQuery.dispose();
    findFocus.dispose();
    workspace.releaseReader(id);
    super.dispose();
  }

  Future<void> save(ReceivedAttachment file) async {
    if (saving != null) return;
    setState(() {
      saving = file.id;
      fileStatus = null;
      fileError = false;
    });
    try {
      final repository = workspace.repository;
      if (repository is! AttachmentRepository) throw StateError('Unavailable');
      final bytes = await (repository as AttachmentRepository).attachment(
        id,
        file,
      );
      if (!mounted) return;
      final saved = await const AttachmentSaver().save(file, bytes);
      if (mounted) {
        setState(
          () => fileStatus = saved ? '${file.name} saved.' : 'Save cancelled.',
        );
      }
    } catch (_) {
      if (mounted) {
        setState(() {
          fileError = true;
          fileStatus =
              'Could not save ${file.name}. Retry Save, or reopen this message if its contents changed.';
        });
      }
    } finally {
      if (mounted) setState(() => saving = null);
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: workspace,
    builder: (context, _) {
      final mail = workspace.mail(id);
      if (mail == null) {
        return Scaffold(
          appBar: AppBar(title: const Text('Message')),
          body: Column(
            children: [
              MailErrorBanner(workspace: workspace),
              const Expanded(
                child: Center(
                  child: Text(
                    'This message moved. Open its destination folder.',
                  ),
                ),
              ),
            ],
          ),
        );
      }
      final scheme = Theme.of(context).colorScheme;
      final parts = mail.body.split('\n>');
      final latest = parts.first;
      final quote = parts.skip(1).join('\n>').trim();
      return CallbackShortcuts(
        bindings: {
          const SingleActivator(LogicalKeyboardKey.keyF, control: true):
              openFind,
          const SingleActivator(LogicalKeyboardKey.keyF, meta: true): openFind,
          if (find.open)
            const SingleActivator(LogicalKeyboardKey.escape): closeFind,
        },
        child: Scaffold(
          appBar: AppBar(
            titleSpacing: 0,
            title: const Text('Message'),
            actions: [
              IconButton(
                tooltip: 'Find in message',
                onPressed: openFind,
                icon: const Icon(Icons.search),
              ),
              IconButton(
                tooltip: 'Refresh mail',
                onPressed: () => workspace.refresh(),
                icon: const Icon(Icons.refresh),
              ),
              IconButton(
                tooltip: 'Archive',
                onPressed: () {
                  act(id, MailAction.archive);
                  Navigator.pop(context);
                },
                icon: const Icon(Icons.archive_outlined),
              ),
              IconButton(
                tooltip: mail.unread ? 'Mark read' : 'Mark unread',
                onPressed: () => act(id, MailAction.read),
                icon: Icon(
                  mail.unread
                      ? Icons.mark_email_read_outlined
                      : Icons.mark_email_unread_outlined,
                ),
              ),
              IconButton(
                tooltip: mail.starred ? 'Unflag' : 'Flag',
                onPressed: () => act(id, MailAction.star),
                icon: Icon(
                  mail.starred ? Icons.flag : Icons.flag_outlined,
                  color: mail.starred ? scheme.error : null,
                ),
              ),
            ],
          ),
          body: Column(
            children: [
              MailErrorBanner(workspace: workspace),
              if (find.open)
                MessageFindBar(
                  find: find,
                  query: findQuery,
                  focus: findFocus,
                  close: closeFind,
                  loading: !mail.bodyLoaded,
                ),
              Expanded(
                child: SingleChildScrollView(
                  padding: const EdgeInsets.all(24),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        mail.subject,
                        style: Theme.of(context).textTheme.headlineSmall
                            ?.copyWith(fontWeight: FontWeight.w600),
                      ),
                      const SizedBox(height: 24),
                      Row(
                        children: [
                          CircleAvatar(
                            backgroundColor: scheme.surfaceContainerHighest,
                            child: Text(mail.sender[0]),
                          ),
                          const SizedBox(width: 12),
                          Expanded(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  mail.sender,
                                  style: const TextStyle(
                                    fontWeight: FontWeight.w600,
                                  ),
                                ),
                                if (!mail.bodyLoaded) ...[
                                  if (workspace.loadingBody(id))
                                    const LinearProgressIndicator()
                                  else
                                    TextButton.icon(
                                      onPressed: () => workspace.loadBody(id),
                                      icon: const Icon(Icons.refresh),
                                      label: const Text('Load message'),
                                    ),
                                  if (workspace.bodyError(id)
                                      case final String error)
                                    Text(
                                      error,
                                      style: TextStyle(color: scheme.error),
                                    ),
                                  const SizedBox(height: 16),
                                ],
                                SelectableText(
                                  mail.address,
                                  style: TextStyle(
                                    color: scheme.onSurfaceVariant,
                                    fontSize: 12,
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 10),
                      Text(
                        '${mail.account} · ${mail.date.toLocal().toString().substring(0, 16)}',
                        style: TextStyle(
                          color: scheme.onSurfaceVariant,
                          fontSize: 12,
                        ),
                      ),
                      const Padding(
                        padding: EdgeInsets.symmetric(vertical: 24),
                        child: Divider(),
                      ),
                      SearchableMessageText(text: latest, block: 0, find: find),
                      if (quote.isNotEmpty &&
                          workspace.preferences.quoteMode != 'Latest only')
                        ExpansionTile(
                          expansionAnimationStyle: AnimationStyle.noAnimation,
                          key: ValueKey(
                            '$id:${workspace.preferences.quoteMode}',
                          ),
                          title: const Text('Quoted history'),
                          onExpansionChanged: (value) {
                            quoteExpanded = value;
                            syncFind();
                          },
                          initiallyExpanded:
                              workspace.preferences.quoteMode == 'Expanded',
                          children: [
                            Padding(
                              padding: const EdgeInsets.all(12),
                              child: SearchableMessageText(
                                text: quote,
                                block: 1,
                                find: find,
                              ),
                            ),
                          ],
                        ),
                      if (mail.fileError case final String error) ...[
                        const SizedBox(height: 16),
                        Semantics(
                          liveRegion: true,
                          child: Text(
                            error,
                            style: TextStyle(color: scheme.error),
                          ),
                        ),
                        TextButton.icon(
                          onPressed: workspace.loadingBody(id)
                              ? null
                              : () => workspace.loadBody(id, force: true),
                          icon: const Icon(Icons.refresh),
                          label: const Text('Reload attachments'),
                        ),
                      ],
                      if (mail.attachments.isNotEmpty) ...[
                        const SizedBox(height: 24),
                        Wrap(
                          spacing: 8,
                          runSpacing: 8,
                          children: mail.files.isNotEmpty
                              ? mail.files
                                    .map(
                                      (file) => OutlinedButton.icon(
                                        onPressed: saving == null
                                            ? () => save(file)
                                            : null,
                                        icon: saving == file.id
                                            ? const SizedBox(
                                                width: 16,
                                                height: 16,
                                                child:
                                                    CircularProgressIndicator(
                                                      strokeWidth: 2,
                                                    ),
                                              )
                                            : const Icon(
                                                Icons.save_alt,
                                                size: 18,
                                              ),
                                        label: Text(
                                          'Save ${file.name} (${file.size} bytes)',
                                        ),
                                      ),
                                    )
                                    .toList()
                              : mail.attachments
                                    .map(
                                      (a) => Chip(
                                        avatar: const Icon(
                                          Icons.attach_file,
                                          size: 16,
                                        ),
                                        label: Text(a),
                                      ),
                                    )
                                    .toList(),
                        ),
                      ],
                      if (fileStatus != null)
                        Padding(
                          padding: const EdgeInsets.only(top: 12),
                          child: Semantics(
                            liveRegion: true,
                            child: Text(
                              fileStatus!,
                              style: TextStyle(
                                color: fileError
                                    ? scheme.error
                                    : scheme.onSurfaceVariant,
                              ),
                            ),
                          ),
                        ),
                      const SizedBox(height: 28),
                      Wrap(
                        spacing: 12,
                        children: [
                          for (final all in [false, true])
                            FilledButton.tonalIcon(
                              onPressed: !mail.bodyLoaded
                                  ? null
                                  : () async {
                                      final draft = await workspace.reply(
                                        id,
                                        all,
                                      );
                                      if (draft != null && context.mounted) {
                                        await Navigator.push(
                                          context,
                                          MaterialPageRoute<void>(
                                            builder: (_) => Composer(
                                              workspace: workspace,
                                              draft: draft,
                                            ),
                                          ),
                                        );
                                      }
                                    },
                              icon: Icon(all ? Icons.reply_all : Icons.reply),
                              label: Text(all ? 'Reply all' : 'Reply'),
                            ),
                          OutlinedButton.icon(
                            onPressed: () => act(id, MailAction.move),
                            icon: const Icon(Icons.drive_file_move_outline),
                            label: const Text('Move'),
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
      );
    },
  );
}
