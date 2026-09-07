import 'mail_action_banner.dart';
import 'mail_error.dart';
import 'dart:async';
import 'package:flutter/material.dart';
import '../model/mail.dart';
import '../data/outgoing.dart';
import 'outbox.dart';
import '../model/workspace.dart';
import 'calendar.dart';
import 'composer.dart';
import 'mail_tile.dart';
import 'preferences.dart';
import 'reader.dart';
import 'theme.dart';

class ShepApp extends StatelessWidget {
  const ShepApp({super.key, required this.workspace});
  final Workspace workspace;
  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: workspace,
    builder: (context, _) => MaterialApp(
      title: 'Shep',
      debugShowCheckedModeBanner: false,
      theme: shepTheme(Brightness.light),
      darkTheme: shepTheme(Brightness.dark),
      themeMode: workspace.preferences.appearance,
      home: Home(workspace: workspace),
    ),
  );
}

class Home extends StatefulWidget {
  const Home({super.key, required this.workspace});
  final Workspace workspace;
  @override
  State<Home> createState() => _HomeState();
}

class _HomeState extends State<Home> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) =>
      w.setForeground(state == AppLifecycleState.resumed);
  int tab = 0;
  bool searching = false;
  final search = TextEditingController();
  Workspace get w => widget.workspace;
  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    search.dispose();
    super.dispose();
  }

  Future<void> act(String id, MailAction action) async {
    if (action == MailAction.move) {
      final target = await showDialog<String>(
        context: context,
        builder: (context) => SimpleDialog(
          title: const Text('Move message'),
          children: w.folders
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
      if (target != null) unawaited(w.action(id, action, destination: target));
    } else {
      unawaited(w.action(id, action));
    }
  }

  void compose([Draft? draft]) {
    unawaited(w.finishReading());
    Navigator.push(
      context,
      MaterialPageRoute<void>(
        builder: (_) => Composer(
          workspace: w,
          draft:
              draft ??
              Draft(id: DateTime.now().microsecondsSinceEpoch.toString()),
        ),
      ),
    );
  }

  Widget folders() => SafeArea(
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.all(24),
          child: Row(
            children: [
              Image.asset(
                Theme.of(context).brightness == Brightness.dark
                    ? 'assets/logo-dark.webp'
                    : 'assets/logo-light.webp',
                width: 36,
              ),
              const SizedBox(width: 12),
              const Text(
                'Shep',
                style: TextStyle(fontSize: 24, fontWeight: FontWeight.w600),
              ),
            ],
          ),
        ),
        Expanded(
          child: ListView(
            children: [
              for (final folder in w.folders)
                ListTile(
                  selected: w.folder == folder && w.account == null,
                  leading: Icon(switch (folder) {
                    'Inbox' => Icons.inbox_outlined,
                    'Archive' => Icons.archive_outlined,
                    'Sent' => Icons.send_outlined,
                    'Drafts' => Icons.edit_note,
                    'Trash' => Icons.delete_outline,
                    _ => Icons.report_outlined,
                  }),
                  title: Text(folder),
                  trailing: folder == 'Inbox' ? Text('${w.unreadCount}') : null,
                  onTap: () {
                    w.navigate(folder);
                    setState(() => tab = 0);
                    Navigator.pop(context);
                  },
                ),
              if (w.repository is OutgoingRepository)
                ListTile(
                  leading: const Icon(Icons.outbox_outlined),
                  title: const Text('Outbox'),
                  onTap: () {
                    Navigator.pop(context);
                    Navigator.push(
                      context,
                      MaterialPageRoute<void>(
                        builder: (_) => OutboxScreen(workspace: w),
                      ),
                    );
                  },
                ),
              const Divider(),
              for (final account in w.accounts)
                ListTile(
                  leading: const Icon(Icons.alternate_email),
                  title: Text(account),
                  onTap: () {
                    w.navigate('Inbox', inAccount: account);
                    setState(() => tab = 0);
                    Navigator.pop(context);
                  },
                ),
            ],
          ),
        ),
        if (w.repository.preview)
          const Padding(
            padding: EdgeInsets.all(24),
            child: Text(
              'Preview workspace\nFictional mail. No network access.',
              style: TextStyle(fontSize: 12),
            ),
          ),
      ],
    ),
  );
  Widget inbox() {
    final scheme = Theme.of(context).colorScheme;
    if (w.folder == 'Drafts') {
      return w.drafts.isEmpty
          ? const Center(child: Text('No saved drafts'))
          : ListView(
              children: w.drafts.values
                  .map(
                    (d) => ListTile(
                      leading: const Icon(Icons.edit_note),
                      title: Text(
                        d.subject.isEmpty ? 'Untitled draft' : d.subject,
                      ),
                      subtitle: Text(d.to),
                      onTap: () => compose(d),
                    ),
                  )
                  .toList(),
            );
    }
    return Column(
      children: [
        if (searching)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
            child: TextField(
              controller: search,
              autofocus: true,
              onChanged: w.search,
              decoration: InputDecoration(
                labelText: 'Search mail',
                prefixIcon: const Icon(Icons.search),
                suffixIcon: IconButton(
                  tooltip: 'Clear search',
                  onPressed: () {
                    search.clear();
                    w.search('');
                  },
                  icon: const Icon(Icons.close),
                ),
              ),
            ),
          ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 6, 10, 8),
          child: Row(
            children: [
              Expanded(
                child: Wrap(
                  spacing: 6,
                  children: [
                    for (final filter in ['All', 'Unread', 'Flagged'])
                      FilterChip(
                        label: Text(
                          filter,
                          style: const TextStyle(fontSize: 12),
                        ),
                        selected: w.filter == filter,
                        showCheckmark: false,
                        onSelected: (_) => w.setFilter(filter),
                      ),
                  ],
                ),
              ),
              IconButton(
                tooltip: w.newestFirst ? 'Oldest first' : 'Newest first',
                onPressed: w.sort,
                icon: const Icon(Icons.swap_vert, size: 20),
              ),
            ],
          ),
        ),
        if (w.selected.isNotEmpty)
          Container(
            color: scheme.primaryContainer.withValues(alpha: .3),
            child: Row(
              children: [
                TextButton(
                  onPressed: w.selectAll,
                  child: Text('${w.selected.length} selected'),
                ),
                const Spacer(),
                IconButton(
                  tooltip: 'Archive selected',
                  onPressed: () {
                    for (final id in List.of(w.selected)) {
                      act(id, MailAction.archive);
                    }
                  },
                  icon: const Icon(Icons.archive_outlined),
                ),
                IconButton(
                  tooltip: 'Mark selected read',
                  onPressed: () {
                    for (final id in List.of(w.selected)) {
                      w.change(id, {'unread': false});
                    }
                  },
                  icon: const Icon(Icons.mark_email_read_outlined),
                ),
              ],
            ),
          ),
        const Divider(),
        Expanded(
          child: w.visible.isEmpty
              ? Center(
                  child: Padding(
                    padding: const EdgeInsets.all(32),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(
                          Icons.inbox_outlined,
                          size: 42,
                          color: scheme.onSurfaceVariant,
                        ),
                        const SizedBox(height: 16),
                        Text(
                          w.query.isNotEmpty
                              ? 'No matching mail'
                              : w.repository.preview
                              ? 'All clear'
                              : w.accounts.isEmpty
                              ? 'Welcome to Shep'
                              : 'All clear',
                          style: Theme.of(context).textTheme.titleLarge,
                        ),
                        const SizedBox(height: 10),
                        Text(
                          w.query.isNotEmpty
                              ? 'Try another sender, subject or phrase.'
                              : w.repository.preview
                              ? 'No messages in this view.'
                              : w.accounts.isEmpty
                              ? 'Add a mail account in Preferences to get started.'
                              : 'No messages in this view. Refresh to check for mail.',
                          textAlign: TextAlign.center,
                        ),
                      ],
                    ),
                  ),
                )
              : RefreshIndicator(
                  onRefresh: w.refresh,
                  child: ListView.builder(
                    itemCount:
                        w.visible.length +
                        (w.visible.length < w.resultCount ? 1 : 0),
                    itemBuilder: (context, i) {
                      if (i == w.visible.length) {
                        return TextButton(
                          onPressed: w.more,
                          child: const Text('Load next 50 messages'),
                        );
                      }
                      final mail = w.visible[i];
                      return MailTile(
                        key: ValueKey('mail-${mail.id}'),
                        mail: mail,
                        workspace: w,
                        act: (action) => act(mail.id, action),
                        open: () {
                          unawaited(w.loadBody(mail.id));
                          w.beginReading(mail.id);
                          Navigator.push(
                            context,
                            MaterialPageRoute<void>(
                              builder: (_) =>
                                  Reader(workspace: w, id: mail.id, act: act),
                            ),
                          );
                        },
                      );
                    },
                  ),
                ),
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: w,
    builder: (context, _) {
      final title = tab == 0
          ? (w.account == null ? w.folder : '${w.account} · ${w.folder}')
          : tab == 1
          ? 'Calendar'
          : 'Preferences';
      return Scaffold(
        drawer: Drawer(child: folders()),
        appBar: AppBar(
          title: Row(
            children: [
              Flexible(child: Text(title, overflow: TextOverflow.ellipsis)),
              if (tab == 0 && w.folder == 'Inbox')
                Padding(
                  padding: const EdgeInsets.only(left: 8),
                  child: Text(
                    '${w.unreadCount}',
                    style: TextStyle(
                      fontSize: 14,
                      color: Theme.of(context).colorScheme.onSurfaceVariant,
                    ),
                  ),
                ),
            ],
          ),
          actions: [
            if (tab == 0) ...[
              IconButton(
                tooltip: 'Search',
                onPressed: () => setState(() => searching = !searching),
                icon: const Icon(Icons.search),
              ),
              IconButton(
                tooltip: w.syncing ? 'Queue refresh' : 'Refresh',
                onPressed: w.refresh,
                icon: w.syncing
                    ? const SizedBox(
                        width: 20,
                        height: 20,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.refresh),
              ),
            ],
          ],
        ),
        body: Column(
          children: [
            if (w.repository.preview)
              Container(
                width: double.infinity,
                padding: const EdgeInsets.symmetric(vertical: 5),
                color: Theme.of(context).colorScheme.surfaceContainerLow,
                child: const Text(
                  'PREVIEW · FICTIONAL DATA',
                  textAlign: TextAlign.center,
                  style: TextStyle(fontSize: 9, letterSpacing: 1.8),
                ),
              ),
            MailErrorBanner(workspace: w),
            Expanded(
              child: tab == 0
                  ? inbox()
                  : tab == 1
                  ? CalendarView(workspace: w)
                  : PreferencesView(workspace: w),
            ),
          ],
        ),
        floatingActionButton: tab == 0
            ? FloatingActionButton.extended(
                onPressed: compose,
                icon: const Icon(Icons.edit_outlined),
                label: const Text('Compose'),
              )
            : null,
        bottomNavigationBar: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            MailActionBanner(workspace: w),
            NavigationBar(
              selectedIndex: tab,
              onDestinationSelected: (value) {
                unawaited(w.finishReading());
                setState(() => tab = value);
              },
              destinations: const [
                NavigationDestination(
                  icon: Icon(Icons.mail_outline),
                  selectedIcon: Icon(Icons.mail),
                  label: 'Mail',
                ),
                NavigationDestination(
                  icon: Icon(Icons.calendar_month_outlined),
                  selectedIcon: Icon(Icons.calendar_month),
                  label: 'Calendar',
                ),
                NavigationDestination(
                  icon: Icon(Icons.tune),
                  label: 'Preferences',
                ),
              ],
            ),
          ],
        ),
      );
    },
  );
}
