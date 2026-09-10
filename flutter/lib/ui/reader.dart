import 'mail_action_banner.dart';
import 'mail_error.dart';
import 'dart:async';
import 'package:url_launcher/url_launcher.dart';
import '../data/formatted_message.dart';
import '../model/formatted_message.dart';
import 'formatted_view.dart';
import 'package:flutter/services.dart';
import '../data/message_search.dart';
import '../model/message_find.dart';
import 'message_find.dart';
import '../data/attachments.dart';
import 'package:flutter/material.dart';
import '../model/mail.dart';
import '../model/workspace.dart';
import 'composer.dart';
import 'format.dart';
import 'icons.dart';
import 'theme.dart';

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
  FormattedMessage? formatted;
  final formattedKey = GlobalKey();
  bool commandsQueued = false, linkOpen = false;
  bool get showQuotes =>
      workspace.preferences.quoteMode != 'Latest only' &&
      (quoteExpanded ?? workspace.preferences.quoteMode == 'Expanded');
  void createFormatted() {
    formatted?.dispose();
    formatted = null;
    final repository = workspace.repository;
    if (repository is FormattedMessageRepository) {
      formatted = FormattedMessage(repository as FormattedMessageRepository, id)
        ..addListener(formattedChanged);
      unawaited(
        formatted!.load(
          dark: Theme.of(context).brightness == Brightness.dark,
          quotes: showQuotes,
        ),
      );
    }
  }

  void formattedChanged() {
    if (!mounted) return;
    syncFind();
    setState(() {});
    queueCommands();
  }

  void queueCommands() {
    if (commandsQueued) return;
    commandsQueued = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      commandsQueued = false;
      if (!mounted) return;
      formatted?.configure(
        dark: Theme.of(context).brightness == Brightness.dark,
        quotes: showQuotes,
      );
      if (formatted?.highlight(find) == true) {
        final target = formattedKey.currentContext;
        if (target != null) {
          unawaited(Scrollable.ensureVisible(target, alignment: 0));
        }
      }
    });
  }

  void runtimeMessage(FormattedMessage source, Map<String, dynamic> value) {
    if (formatted != source ||
        !source.receive(value) ||
        !source.html ||
        ModalRoute.of(context)?.isCurrent != true) {
      return;
    }
    if (value['type'] == 'link' && value['url'] is String) {
      unawaited(reviewLink(value['url'] as String));
    } else if (value['type'] == 'shortcut') {
      if (value['key'] == 'Control+f' || value['key'] == 'Meta+f') openFind();
      if (value['key'] == 'Escape') {
        if (find.open) {
          closeFind();
        } else {
          unawaited(Navigator.maybePop(context));
        }
      }
    }
  }

  Future<void> reviewLink(String value) async {
    final url = Uri.tryParse(value);
    if (linkOpen ||
        url == null ||
        !['https', 'http', 'mailto'].contains(url.scheme) ||
        url.userInfo.isNotEmpty) {
      return;
    }
    linkOpen = true;
    String? status;
    try {
      await showDialog<void>(
        context: context,
        builder: (context) => StatefulBuilder(
          builder: (context, update) => AlertDialog(
            title: const Text('Message link'),
            content: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SelectableText(url.toString()),
                if (status != null)
                  Semantics(liveRegion: true, child: Text(status!)),
              ],
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(context),
                child: const Text('Close'),
              ),
              TextButton(
                onPressed: () async {
                  try {
                    await Clipboard.setData(
                      ClipboardData(text: url.toString()),
                    );
                    if (context.mounted) {
                      update(() => status = 'Address copied.');
                    }
                  } catch (_) {
                    if (context.mounted) {
                      update(
                        () => status = 'Select and copy the address above.',
                      );
                    }
                  }
                },
                child: const Text('Copy address'),
              ),
              FilledButton(
                onPressed: () async {
                  try {
                    final opened = await launchUrl(
                      url,
                      mode: LaunchMode.externalApplication,
                    );
                    if (context.mounted) {
                      if (opened) {
                        Navigator.pop(context);
                      } else {
                        update(
                          () => status =
                              'No application could open this link. Copy the address instead.',
                        );
                      }
                    }
                  } catch (_) {
                    if (context.mounted) {
                      update(
                        () => status =
                            'Could not open this link. Copy the address instead.',
                      );
                    }
                  }
                },
                child: const Text('Open link'),
              ),
            ],
          ),
        ),
      );
    } finally {
      linkOpen = false;
    }
  }

  void syncFind() {
    final mail = workspace.mail(id);
    final mode = workspace.preferences.quoteMode;
    if (mode != quoteMode) {
      quoteMode = mode;
      quoteExpanded = null;
    }
    final document = formatted;
    if (document?.html == true) {
      find.setSource('$id:html:${document!.generation}', document.blocks);
      queueCommands();
      return;
    }
    final parts = (document?.prepared?.text ?? mail?.body ?? '').split('\n>');
    final quote = parts.skip(1).join('\n>').trim();
    find.setSource('$id:plain', [
      parts.first,
      if (quote.isNotEmpty &&
          mode != 'Latest only' &&
          (quoteExpanded ?? mode == 'Expanded'))
        quote,
    ]);
  }

  void findChanged() {
    if (mounted) {
      setState(() {});
      queueCommands();
    }
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
      createFormatted();
      syncFind();
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (formatted == null &&
        workspace.repository is FormattedMessageRepository) {
      createFormatted();
    }
    queueCommands();
  }

  @override
  void dispose() {
    workspace.removeListener(syncFind);
    formatted?.dispose();
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
  Widget build(BuildContext context) => PopScope(
    onPopInvokedWithResult: (didPop, result) {
      if (didPop) unawaited(workspace.finishReading(only: id));
    },
    child: buildReader(context),
  );

  Widget actionIcon(String icon, bool busy) => SizedBox(
    width: 18,
    height: 18,
    child: busy
        ? const ExcludeSemantics(
            child: CircularProgressIndicator(strokeWidth: 2),
          )
        : ShepIcon(icon, size: 18),
  );

  Widget readerActions(
    BuildContext context,
    List<Widget> actions,
  ) => DecoratedBox(
    decoration: BoxDecoration(
      color: ShepColors.of(context).surface,
      border: Border(top: BorderSide(color: ShepColors.of(context).border)),
    ),
    child: Padding(
      padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
      child: LayoutBuilder(
        builder: (context, constraints) {
          final scale = MediaQuery.textScalerOf(context).scale(14) / 14;
          final columns =
              ((constraints.maxWidth + 8) /
                      (120 + 56 * (scale - 1).clamp(0, double.infinity)))
                  .floor()
                  .clamp(1, actions.length);
          final width = (constraints.maxWidth - 8 * (columns - 1)) / columns;
          final style = ButtonStyle(
            visualDensity: VisualDensity.standard,
            minimumSize: const WidgetStatePropertyAll(Size(0, 44)),
            padding: const WidgetStatePropertyAll(
              EdgeInsets.symmetric(horizontal: 10, vertical: 10),
            ),
            iconSize: const WidgetStatePropertyAll(18),
          );
          return FilledButtonTheme(
            data: FilledButtonThemeData(
              style: FilledButtonTheme.of(context).style?.merge(style) ?? style,
            ),
            child: OutlinedButtonTheme(
              data: OutlinedButtonThemeData(
                style:
                    OutlinedButtonTheme.of(context).style?.merge(style) ??
                    style,
              ),
              child: Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  for (final action in actions)
                    SizedBox(width: width, child: action),
                ],
              ),
            ),
          );
        },
      ),
    ),
  );

  Widget buildReader(BuildContext context) => ListenableBuilder(
    listenable: workspace,
    builder: (context, _) {
      final mail = workspace.mail(id);
      if (mail == null) {
        return Scaffold(
          appBar: AppBar(title: const Text('Message')),
          bottomNavigationBar: SafeArea(
            top: false,
            child: MailActionBanner(workspace: workspace),
          ),
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
      final c = ShepColors.of(context);
      final document = formatted;
      final html = document?.html == true;
      final parts = (document?.prepared?.text ?? mail.body).split('\n>');
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
          backgroundColor: c.surface,
          bottomNavigationBar: SafeArea(
            top: false,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                MailActionBanner(workspace: workspace),
                readerActions(context, [
                  for (final all in [false, true])
                    () {
                      Future<void> reply() async {
                        final draft = await workspace.reply(id, all);
                        if (draft != null && context.mounted) {
                          unawaited(workspace.finishReading());
                          await Navigator.push(
                            context,
                            MaterialPageRoute<void>(
                              builder: (_) =>
                                  Composer(workspace: workspace, draft: draft),
                            ),
                          );
                        }
                      }

                      final icon = ShepIcon(all ? 'reply-all' : 'reply');
                      final label = Text(all ? 'Reply all' : 'Reply');
                      final onPressed = mail.bodyLoaded ? reply : null;
                      return all
                          ? OutlinedButton.icon(
                              onPressed: onPressed,
                              icon: icon,
                              label: label,
                            )
                          : FilledButton.icon(
                              onPressed: onPressed,
                              icon: icon,
                              label: label,
                            );
                    }(),
                  OutlinedButton.icon(
                    onPressed: workspace.isForwarding(id)
                        ? null
                        : () async {
                            final source = id;
                            final draft = await workspace.forward(source);
                            if (draft != null &&
                                context.mounted &&
                                widget.id == source &&
                                (ModalRoute.of(context)?.isCurrent ?? false)) {
                              unawaited(workspace.finishReading());
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
                    icon: actionIcon('forward', workspace.isForwarding(id)),
                    label: Text(
                      'Forward',
                      semanticsLabel: workspace.isForwarding(id)
                          ? 'Preparing forward…'
                          : 'Forward',
                    ),
                  ),
                  OutlinedButton.icon(
                    onPressed: workspace.isPrinting(id)
                        ? null
                        : () => workspace.printMessage(
                            id,
                            plain: formatted?.plain ?? false,
                          ),
                    icon: actionIcon('print', workspace.isPrinting(id)),
                    label: Text(
                      'Print',
                      semanticsLabel: workspace.isPrinting(id)
                          ? 'Preparing print…'
                          : 'Print',
                    ),
                  ),
                  OutlinedButton.icon(
                    onPressed: () => act(id, MailAction.move),
                    icon: const ShepIcon('move'),
                    label: const Text('Move'),
                  ),
                ]),
              ],
            ),
          ),
          appBar: AppBar(
            titleSpacing: 0,
            backgroundColor: c.surface,
            bottom: const PreferredSize(
              preferredSize: Size.fromHeight(1),
              child: Divider(height: 1),
            ),
            title: const Text('Message'),
            actions: [
              IconButton(
                tooltip: 'Find in message',
                onPressed: openFind,
                icon: const ShepIcon('search'),
              ),
              IconButton(
                tooltip: 'Refresh mail',
                onPressed: () => workspace.refresh(),
                icon: const ShepIcon('sync'),
              ),
              IconButton(
                tooltip: 'Archive',
                onPressed: () {
                  act(id, MailAction.archive);
                  Navigator.pop(context);
                },
                icon: const ShepIcon('archive'),
              ),
              IconButton(
                tooltip: mail.unread ? 'Mark read' : 'Mark unread',
                onPressed: () => act(id, MailAction.read),
                icon: ShepIcon(mail.unread ? 'mail-open' : 'mail'),
              ),
              IconButton(
                tooltip: mail.starred ? 'Unflag' : 'Flag',
                onPressed: () => act(id, MailAction.star),
                style: mail.starred
                    ? ButtonStyle(
                        side: WidgetStatePropertyAll(
                          BorderSide(color: c.flag, width: 1.5),
                        ),
                      )
                    : null,
                icon: ShepIcon('flag', color: mail.starred ? c.flag : null),
              ),
              const SizedBox(width: 6),
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
                  loading: !mail.bodyLoaded && document?.prepared == null,
                ),
              Expanded(
                child: LayoutBuilder(
                  builder: (context, constraints) => SingleChildScrollView(
                    padding: const EdgeInsets.fromLTRB(22, 26, 22, 22),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          mail.subject,
                          style: TextStyle(
                            fontSize: ShepText.heading,
                            fontWeight: FontWeight.w600,
                            color: c.text,
                          ),
                        ),
                        const SizedBox(height: 20),
                        Row(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Container(
                              width: 41,
                              height: 41,
                              alignment: Alignment.center,
                              decoration: BoxDecoration(
                                color: avatarColors(0).$1,
                                shape: BoxShape.circle,
                              ),
                              child: Text(
                                avatarInitials(mail.sender),
                                style: TextStyle(
                                  fontSize: 15,
                                  fontWeight: FontWeight.w600,
                                  color: avatarColors(0).$2,
                                ),
                              ),
                            ),
                            const SizedBox(width: 12),
                            Expanded(
                              child: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  Text(
                                    mail.sender,
                                    style: TextStyle(
                                      fontSize: ShepText.body,
                                      fontWeight: FontWeight.w600,
                                      color: c.text,
                                    ),
                                  ),
                                  if (!mail.bodyLoaded) ...[
                                    if (workspace.loadingBody(id))
                                      const LinearProgressIndicator()
                                    else
                                      TextButton.icon(
                                        onPressed: () => workspace.loadBody(id),
                                        icon: const ShepIcon('sync'),
                                        label: const Text('Load message'),
                                      ),
                                    if (workspace.bodyError(id)
                                        case final String error)
                                      Text(
                                        error,
                                        style: TextStyle(color: c.flag),
                                      ),
                                    const SizedBox(height: 16),
                                  ],
                                  const SizedBox(height: 5),
                                  SelectableText(
                                    mail.address,
                                    style: TextStyle(
                                      color: c.muted,
                                      fontSize: ShepText.caption,
                                    ),
                                  ),
                                  const SizedBox(height: 5),
                                  Text(
                                    'To: ${mail.account}',
                                    style: TextStyle(
                                      color: c.muted,
                                      fontSize: ShepText.caption,
                                    ),
                                  ),
                                ],
                              ),
                            ),
                            const SizedBox(width: 12),
                            Text(
                              readerDate(mail.date),
                              textAlign: TextAlign.right,
                              style: TextStyle(
                                color: c.muted,
                                fontSize: ShepText.caption,
                                height: 1.8,
                              ),
                            ),
                          ],
                        ),
                        const Padding(
                          padding: EdgeInsets.symmetric(vertical: 20),
                          child: Divider(),
                        ),
                        if (document != null &&
                            (document.loading ||
                                document.error != null ||
                                document.prepared?.document != null)) ...[
                          Wrap(
                            spacing: 8,
                            runSpacing: 8,
                            crossAxisAlignment: WrapCrossAlignment.center,
                            children: [
                              SegmentedButton<bool>(
                                segments: const [
                                  ButtonSegment(
                                    value: false,
                                    label: Text('Formatted'),
                                  ),
                                  ButtonSegment(
                                    value: true,
                                    label: Text('Plain text'),
                                  ),
                                ],
                                selected: {document.plain},
                                onSelectionChanged: (value) =>
                                    document.setPlain(value.single),
                              ),
                              if (html &&
                                  document.hasQuotes &&
                                  workspace.preferences.quoteMode !=
                                      'Latest only')
                                OutlinedButton(
                                  onPressed: () {
                                    quoteExpanded = !showQuotes;
                                    setState(() {});
                                    queueCommands();
                                  },
                                  child: Text(
                                    showQuotes
                                        ? 'Hide quoted history'
                                        : 'Show quoted history',
                                  ),
                                ),
                            ],
                          ),
                          if (document.loading)
                            const LinearProgressIndicator(
                              semanticsLabel: 'Preparing formatted message',
                            ),
                          if (document.error != null) ...[
                            Semantics(
                              liveRegion: true,
                              child: Text(
                                document.error!,
                                style: TextStyle(color: scheme.error),
                              ),
                            ),
                            TextButton.icon(
                              onPressed: () => document.load(
                                dark:
                                    Theme.of(context).brightness ==
                                    Brightness.dark,
                                quotes: showQuotes,
                              ),
                              icon: const ShepIcon('sync'),
                              label: const Text('Retry formatted message'),
                            ),
                          ],
                          const SizedBox(height: 12),
                        ],
                        if (html && document!.prepared!.remoteImages.isNotEmpty)
                          Padding(
                            padding: const EdgeInsets.only(bottom: 8),
                            child: Text(
                              '${document.prepared!.remoteImages.length} remote image${document.prepared!.remoteImages.length == 1 ? '' : 's'} blocked.',
                              style: TextStyle(
                                color: scheme.onSurfaceVariant,
                                fontSize: 12,
                              ),
                            ),
                          ),
                        if (html)
                          for (final issue in document!.prepared!.issues)
                            Text(
                              issue,
                              style: TextStyle(color: scheme.onSurfaceVariant),
                            ),
                        if (document?.prepared?.document != null &&
                            document?.error == null)
                          Offstage(
                            key: ValueKey('formatted:${document!.generation}'),
                            offstage: !html,
                            child: SizedBox(
                              key: formattedKey,
                              height: (constraints.maxHeight * 0.85).clamp(
                                160.0,
                                720.0,
                              ),
                              width: double.infinity,
                              child: ClipRRect(
                                borderRadius: BorderRadius.circular(8),
                                child: FormattedView(
                                  document: document.prepared!.document!,
                                  commands: document.commands.stream,
                                  onMessage: (value) =>
                                      runtimeMessage(document, value),
                                  onError: () => document.displayError(
                                    document.generation,
                                  ),
                                ),
                              ),
                            ),
                          ),
                        if (!html)
                          SearchableMessageText(
                            text: latest,
                            block: 0,
                            find: find,
                          ),
                        if (!html &&
                            quote.isNotEmpty &&
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
                            initiallyExpanded: showQuotes,
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
                            icon: const ShepIcon('sync'),
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
                                              : const ShepIcon(
                                                  'download',
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
                                          avatar: const ShepIcon(
                                            'clip',
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
                      ],
                    ),
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
