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
import 'controls.dart';
import 'icons.dart';
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

  /// Sidebar row styled like the desktop nav and folder buttons.
  Widget sidebarItem(
    String label, {
    required String icon,
    required VoidCallback onTap,
    bool selected = false,
    bool nav = false,
    Widget? trailing,
  }) {
    final c = ShepColors.of(context);
    return Material(
      color: selected ? c.tint : Colors.transparent,
      borderRadius: BorderRadius.circular(ShepRadius.control),
      child: InkWell(
        onTap: onTap,
        hoverColor: c.subtle,
        borderRadius: BorderRadius.circular(ShepRadius.control),
        child: Container(
          constraints: const BoxConstraints(minHeight: 44),
          padding: EdgeInsets.symmetric(
            horizontal: nav ? 12 : 10,
            vertical: nav ? 11 : 9,
          ),
          child: Row(
            children: [
              ShepIcon(
                icon,
                size: nav ? 20 : 18,
                color: selected ? c.accent : c.muted,
              ),
              SizedBox(width: nav ? 11 : 9),
              Expanded(
                child: Text(
                  label,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: nav ? ShepText.body : ShepText.secondary,
                    color: selected ? c.accent : c.text,
                  ),
                ),
              ),
              ?trailing,
            ],
          ),
        ),
      ),
    );
  }

  Widget folders() {
    final c = ShepColors.of(context);
    void select(VoidCallback go) {
      go();
      setState(() => tab = 0);
      Navigator.pop(context);
    }

    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(14, 19, 14, 14),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(7, 6, 7, 20),
              child: Row(
                children: [
                  Image.asset(
                    Theme.of(context).brightness == Brightness.dark
                        ? 'assets/logo-dark.webp'
                        : 'assets/logo-light.webp',
                    width: 38,
                    height: 38,
                  ),
                  const SizedBox(width: 10),
                  Text(
                    'shep',
                    style: TextStyle(
                      fontSize: 29,
                      fontWeight: FontWeight.w600,
                      color: c.text,
                    ),
                  ),
                ],
              ),
            ),
            Expanded(
              child: ListView(
                padding: EdgeInsets.zero,
                children: [
                  sidebarItem(
                    'Mail',
                    icon: 'mail',
                    nav: true,
                    selected: tab == 0,
                    onTap: () {
                      setState(() => tab = 0);
                      Navigator.pop(context);
                    },
                  ),
                  const SizedBox(height: 3),
                  sidebarItem(
                    'Calendar',
                    icon: 'calendar',
                    nav: true,
                    selected: tab == 1,
                    onTap: () {
                      unawaited(w.finishReading());
                      setState(() => tab = 1);
                      Navigator.pop(context);
                    },
                  ),
                  const SizedBox(height: 14),
                  FilledButton.icon(
                    onPressed: () {
                      Navigator.pop(context);
                      compose();
                    },
                    icon: const ShepIcon(
                      'compose',
                      size: 20,
                      color: Colors.white,
                    ),
                    label: const Text('New message'),
                    style: const ButtonStyle(
                      padding: WidgetStatePropertyAll(
                        EdgeInsets.symmetric(horizontal: 14, vertical: 12),
                      ),
                      alignment: Alignment.centerLeft,
                    ),
                  ),
                  const SizedBox(height: 14),
                  for (final folder in w.folders)
                    sidebarItem(
                      folder == 'Inbox' && w.unreadCount > 0
                          ? 'Inbox (${w.unreadCount})'
                          : folder,
                      icon: switch (folder) {
                        'Inbox' => 'inbox',
                        'Archive' => 'archive',
                        'Sent' => 'send',
                        'Drafts' => 'file',
                        'Trash' => 'trash',
                        _ => 'shield',
                      },
                      selected:
                          tab == 0 && w.folder == folder && w.account == null,
                      onTap: () => select(() => w.navigate(folder)),
                    ),
                  if (w.repository is OutgoingRepository)
                    sidebarItem(
                      'Outbox',
                      icon: 'outbox',
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
                  const SizedBox(height: 17),
                  for (final account in w.accounts)
                    sidebarItem(
                      account,
                      icon: 'mail',
                      selected: tab == 0 && w.account == account,
                      onTap: () =>
                          select(() => w.navigate('Inbox', inAccount: account)),
                    ),
                ],
              ),
            ),
            Divider(color: c.border),
            const SizedBox(height: 12),
            sidebarItem(
              'Preferences',
              icon: 'settings',
              nav: true,
              selected: tab == 2,
              onTap: () {
                unawaited(w.finishReading());
                setState(() => tab = 2);
                Navigator.pop(context);
              },
            ),
            if (w.repository.preview)
              Padding(
                padding: const EdgeInsets.fromLTRB(12, 12, 12, 0),
                child: Text(
                  'Preview workspace\nFictional mail. No network access.',
                  style: TextStyle(fontSize: ShepText.small, color: c.muted),
                ),
              ),
          ],
        ),
      ),
    );
  }

  /// Desktop-style badge: tinted pill with bold accent capitals.
  Widget badge(String label) {
    final c = ShepColors.of(context);
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      decoration: BoxDecoration(
        color: c.tint,
        borderRadius: BorderRadius.circular(ShepRadius.badge),
      ),
      child: Text(
        label,
        style: TextStyle(
          fontSize: ShepText.caption,
          fontWeight: FontWeight.w600,
          color: c.accent,
        ),
      ),
    );
  }

  /// Filter choice styled like the desktop selected/outline buttons.
  Widget filterChip(String filter) {
    final c = ShepColors.of(context);
    final selected = w.filter == filter;
    return FilterChip(
      label: Text(filter),
      selected: selected,
      showCheckmark: false,
      backgroundColor: c.surface,
      selectedColor: c.tint,
      side: BorderSide(color: selected ? c.tint : c.border),
      labelStyle: TextStyle(
        fontSize: ShepText.secondary,
        color: selected ? c.accent : c.text,
      ),
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 10),
      materialTapTargetSize: MaterialTapTargetSize.padded,
      onSelected: (_) => w.setFilter(filter),
    );
  }

  Widget inbox() {
    final c = ShepColors.of(context);
    if (w.folder == 'Drafts') {
      return w.drafts.isEmpty
          ? Center(
              child: Text('No saved drafts', style: TextStyle(color: c.muted)),
            )
          : ListView(
              children: w.drafts.values
                  .map(
                    (d) => ListTile(
                      leading: const ShepIcon('file'),
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
    return Container(
      color: c.surface,
      child: Column(
        children: [
          if (searching)
            Padding(
              padding: const EdgeInsets.fromLTRB(18, 14, 18, 0),
              child: TextField(
                controller: search,
                autofocus: true,
                onChanged: w.search,
                style: const TextStyle(fontSize: ShepText.body),
                decoration: InputDecoration(
                  hintText: 'Search conversations…',
                  contentPadding: const EdgeInsets.symmetric(
                    horizontal: 12,
                    vertical: 11,
                  ),
                  prefixIcon: const Center(child: ShepIcon('search', size: 18)),
                  prefixIconConstraints: const BoxConstraints(
                    minWidth: 40,
                    minHeight: 40,
                    maxWidth: 40,
                  ),
                  suffixIcon: IconButton(
                    tooltip: 'Clear search',
                    onPressed: () {
                      search.clear();
                      w.search('');
                    },
                    icon: const ShepIcon('close', size: 18),
                  ),
                ),
              ),
            ),
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 10, 10, 10),
            child: Row(
              children: [
                Expanded(
                  child: Wrap(
                    spacing: 6,
                    runSpacing: 6,
                    children: [
                      for (final filter in ['All', 'Unread', 'Flagged'])
                        filterChip(filter),
                    ],
                  ),
                ),
                IconButton(
                  tooltip: w.newestFirst ? 'Oldest first' : 'Newest first',
                  onPressed: w.sort,
                  icon: const ShepIcon('sort', size: 18),
                ),
              ],
            ),
          ),
          if (w.selected.isNotEmpty)
            Container(
              color: c.tint,
              padding: const EdgeInsets.symmetric(horizontal: 6),
              child: Row(
                children: [
                  TextButton(
                    onPressed: w.selectAll,
                    style: TextButton.styleFrom(foregroundColor: c.accent),
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
                    icon: const ShepIcon('archive'),
                  ),
                  IconButton(
                    tooltip: 'Mark selected read',
                    onPressed: () {
                      for (final id in List.of(w.selected)) {
                        w.change(id, {'unread': false});
                      }
                    },
                    icon: const ShepIcon('mail-open'),
                  ),
                ],
              ),
            ),
          const Divider(),
          Expanded(
            child: w.visible.isEmpty
                ? EmptyState(
                    icon: 'search',
                    title: w.query.isNotEmpty
                        ? 'No matching mail'
                        : w.repository.preview
                        ? 'All clear'
                        : w.accounts.isEmpty
                        ? 'Welcome to Shep'
                        : 'All clear',
                    message: w.query.isNotEmpty
                        ? 'Try another sender, subject or phrase.'
                        : w.repository.preview
                        ? 'No messages in this view.'
                        : w.accounts.isEmpty
                        ? 'Add a mail account in Preferences to get started.'
                        : 'No messages in this view. Refresh to check for mail.',
                  )
                : RefreshIndicator(
                    onRefresh: w.refresh,
                    color: c.accent,
                    backgroundColor: c.surface,
                    child: ListView.builder(
                      itemCount:
                          w.visible.length +
                          (w.visible.length < w.resultCount ? 1 : 0),
                      itemBuilder: (context, i) {
                        if (i == w.visible.length) {
                          return Padding(
                            padding: const EdgeInsets.all(8),
                            child: TextButton(
                              onPressed: w.more,
                              child: const Text('Load next 50 messages'),
                            ),
                          );
                        }
                        final mail = w.visible[i];
                        return MailTile(
                          key: ValueKey('mail-${mail.id}'),
                          mail: mail,
                          index: i,
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
      ),
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
      final c = ShepColors.of(context);
      return Scaffold(
        drawer: Drawer(child: folders()),
        appBar: AppBar(
          titleSpacing: 0,
          title: Row(
            children: [
              Flexible(child: Text(title, overflow: TextOverflow.ellipsis)),
              if (tab == 0 && w.folder == 'Inbox')
                Flexible(
                  child: Padding(
                    padding: const EdgeInsets.only(left: 12),
                    child: Text(
                      '${w.unreadCount} unread',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: ShepText.secondary,
                        fontWeight: FontWeight.w400,
                        color: c.muted,
                      ),
                    ),
                  ),
                ),
            ],
          ),
          actions: [
            if (w.repository.preview)
              Padding(
                padding: const EdgeInsets.only(right: 6),
                child: badge('PREVIEW'),
              ),
            if (tab == 0) ...[
              IconButton(
                tooltip: 'Search',
                onPressed: () => setState(() => searching = !searching),
                icon: const ShepIcon('search', size: 20),
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
                    : const ShepIcon('sync', size: 20),
              ),
            ],
            const SizedBox(width: 6),
          ],
        ),
        body: Column(
          children: [
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
                icon: const ShepIcon('compose', size: 20, color: Colors.white),
                label: const Text('New message'),
              )
            : null,
        bottomNavigationBar: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            MailActionBanner(workspace: w),
            DecoratedBox(
              decoration: BoxDecoration(
                border: Border(top: BorderSide(color: c.border)),
              ),
              child: NavigationBar(
                selectedIndex: tab,
                onDestinationSelected: (value) {
                  unawaited(w.finishReading());
                  setState(() => tab = value);
                },
                destinations: const [
                  NavigationDestination(icon: ShepIcon('mail'), label: 'Mail'),
                  NavigationDestination(
                    icon: ShepIcon('calendar'),
                    label: 'Calendar',
                  ),
                  NavigationDestination(
                    icon: ShepIcon('settings'),
                    label: 'Preferences',
                  ),
                ],
              ),
            ),
          ],
        ),
      );
    },
  );
}
