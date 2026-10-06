import 'account_removal.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart' show listEquals;
import 'cal_dav_connections.dart';
import '../model/mail.dart';
import '../model/workspace.dart';
import 'controls.dart';
import 'icons.dart';
import 'theme.dart';
import 'account_setup.dart';
import 'sent_preferences.dart';
import '../data/accounts.dart';
import 'google_connection.dart';
import 'profile_discovery.dart';
import 'profile_sync.dart';
import '../model/preferences_search.dart';
import 'preferences_catalogue.dart';
import 'dart:async';
import '../data/preferences_search_factory_stub.dart'
    if (dart.library.io) '../data/preferences_search_factory_native.dart';

class PreferencesView extends StatefulWidget {
  const PreferencesView({super.key, required this.workspace, this.matcher});
  final Workspace workspace;
  final PreferenceSearchMatcher? matcher;
  @override
  State<PreferencesView> createState() => _PreferencesViewState();
}

class _PreferencesViewState extends State<PreferencesView> {
  final search = TextEditingController();
  final controls = GlobalKey();
  String query = '';
  String? revealed;
  String? revealedTarget;
  int revealGeneration = 0;
  int searchGeneration = 0;
  String? searchedQuery, searchError;
  List<PreferenceSearchEntry>? searchedCatalogue;
  bool searchRunning = false;
  (List<PreferenceSearchEntry>, String, int)? pendingSearch;
  List<PreferenceSearchEntry> results = [];
  Workspace get workspace => widget.workspace;
  PreferenceSearchMatcher get matcher {
    if (widget.matcher case final custom?) return custom;
    return preferenceSearchMatcher(workspace.repository);
  }

  @override
  void didUpdateWidget(PreferencesView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.workspace != widget.workspace ||
        oldWidget.matcher != widget.matcher) {
      searchedCatalogue = null;
      searchGeneration++;
    }
  }

  void requestSearch(List<PreferenceSearchEntry> entries) {
    if (query == searchedQuery && listEquals(entries, searchedCatalogue)) {
      return;
    }
    final frozen = List<PreferenceSearchEntry>.unmodifiable(entries);
    searchedQuery = query;
    searchedCatalogue = frozen;
    final generation = ++searchGeneration;
    results = [];
    searchError = null;
    pendingSearch = query.trim().isEmpty ? null : (frozen, query, generation);
    if (!searchRunning && pendingSearch != null) unawaited(runSearch());
  }

  Future<void> runSearch() async {
    searchRunning = true;
    while (mounted && pendingSearch != null) {
      final (entries, query, generation) = pendingSearch!;
      pendingSearch = null;
      try {
        final found = await matcher.match(entries, query);
        if (mounted && generation == searchGeneration) {
          setState(() => results = found);
        }
      } on Object {
        if (mounted && generation == searchGeneration) {
          setState(
            () => searchError =
                'Preferences search could not complete. Retry search.',
          );
        }
      }
    }
    searchRunning = false;
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    search.dispose();
    super.dispose();
  }

  void reveal(PreferenceSearchEntry entry) {
    final generation = ++revealGeneration;
    FocusManager.instance.primaryFocus?.unfocus();
    search.clear();
    setState(() {
      query = '';
      revealed = entry.label;
      revealedTarget = entry.target;
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || generation != revealGeneration) return;
      Element? target;
      void visit(Element element) {
        if (element.widget.key == ValueKey(entry.target)) target = element;
        element.visitChildren(visit);
      }

      controls.currentContext?.visitChildElements(visit);
      if (target case final element?) {
        Scrollable.ensureVisible(element, alignment: .15);
        void focusField(Element child) {
          if (child.widget case final EditableText field) {
            field.focusNode.requestFocus();
          }
          child.visitChildren(focusField);
        }

        if (element.widget is TextField) element.visitChildren(focusField);
      } else {
        setState(
          () => revealed = 'This control is no longer available. Search again.',
        );
      }
    });
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([
      workspace,
      ?workspace.google,
      ?workspace.profileDiscovery,
    ]),
    builder: (context, _) => buildPreferences(context),
  );

  Widget buildPreferences(BuildContext context) {
    requestSearch(preferencesCatalogue(workspace));
    return Material(
      type: MaterialType.transparency,
      child: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 12, 18, 8),
            child: TextField(
              key: const ValueKey('preferences-search'),
              controller: search,
              maxLength: 256,
              decoration: InputDecoration(
                counterText: '',
                labelText: 'Search preferences',
                prefixIcon: const Center(
                  widthFactor: 1,
                  heightFactor: 1,
                  child: ShepIcon('search'),
                ),
                suffixIcon: query.isEmpty
                    ? null
                    : IconButton(
                        tooltip: 'Clear search',
                        onPressed: () {
                          revealGeneration++;
                          search.clear();
                          setState(() => query = '');
                        },
                        icon: const ShepIcon('close'),
                      ),
              ),
              onChanged: (value) => setState(() {
                revealGeneration++;
                query = value;
                revealed = null;
                revealedTarget = null;
              }),
            ),
          ),
          if (revealed != null)
            Semantics(
              liveRegion: true,
              child: Padding(
                padding: const EdgeInsets.fromLTRB(18, 0, 18, 8),
                child: Text('Showing: $revealed'),
              ),
            ),
          Expanded(
            child: Stack(
              children: [
                Offstage(
                  offstage: query.trim().isNotEmpty,
                  child: buildControls(context),
                ),
                if (query.trim().isNotEmpty)
                  searchError != null
                      ? Center(
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              Text(searchError!),
                              TextButton(
                                onPressed: () =>
                                    setState(() => searchedCatalogue = null),
                                child: const Text('Retry search'),
                              ),
                            ],
                          ),
                        )
                      : searchRunning && results.isEmpty
                      ? const Center(child: Text('Searching preferences…'))
                      : results.isEmpty
                      ? const Center(
                          child: Text(
                            'No preferences found. Try another word.',
                          ),
                        )
                      : ListView(
                          key: const ValueKey('preferences-results'),
                          children: [
                            for (final entry in results)
                              ListTile(
                                key: ValueKey(
                                  'preferences-result-${entry.target}-${entry.label}',
                                ),
                                title: Text(entry.label),
                                subtitle: Text(
                                  '${entry.section}${entry.description.isEmpty ? '' : ' · ${entry.description}'}',
                                ),
                                trailing: const ShepIcon('chevron'),
                                onTap: () => reveal(entry),
                              ),
                          ],
                        ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget buildControls(BuildContext context) {
    final p = workspace.preferences;
    final c = ShepColors.of(context);
    Widget section(String title, List<Widget> children) => Padding(
      padding: const EdgeInsets.only(bottom: 18),
      child: SettingsCard(title: title, children: children),
    );
    Widget swipe(String label, MailAction value, bool left) => ListTile(
      key: ValueKey(left ? 'preference-swipe-left' : 'preference-swipe-right'),
      title: Text(label),
      leading: ShepIcon(left ? 'swipe-left' : 'swipe-right'),
      trailing: pickList<MailAction>(
        context,
        value: value,
        onChanged: (v) {
          if (v != null) {
            workspace.savePreferences(
              left
                  ? workspace.preferences.copy(leftSwipe: v)
                  : workspace.preferences.copy(rightSwipe: v),
            );
          }
        },
        items: MailAction.values
            .map(
              (a) => DropdownMenuItem(
                value: a,
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    actionIcon(a.name, size: 16),
                    const SizedBox(width: 8),
                    Text(a.label),
                  ],
                ),
              ),
            )
            .toList(),
      ),
    );
    return SingleChildScrollView(
      key: const ValueKey('preferences-list'),
      padding: const EdgeInsets.all(18),
      child: Column(
        key: controls,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(2, 0, 2, 18),
            child: Text(
              'Make Shep feel like home.',
              style: TextStyle(fontSize: ShepText.secondary, color: c.muted),
            ),
          ),
          if (workspace.savingPreferences ||
              workspace.preferenceSaveError != null)
            Padding(
              key: const ValueKey('preference-save-status'),
              padding: const EdgeInsets.only(bottom: 18),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      workspace.savingPreferences
                          ? 'Saving preferences…'
                          : workspace.preferenceSaveError!,
                    ),
                  ),
                  if (workspace.preferenceSaveError != null)
                    TextButton(
                      onPressed: workspace.savingPreferences
                          ? null
                          : () => workspace.savePreferences(
                              workspace.preferences,
                            ),
                      child: const Text('Retry save'),
                    ),
                ],
              ),
            ),
          section('Appearance', [
            ListTile(
              key: const ValueKey('preference-theme'),
              title: const Text('Theme'),
              leading: const ShepIcon('sun'),
              trailing: pickList<ThemeMode>(
                context,
                value: p.appearance,
                onChanged: (v) {
                  if (v != null) {
                    workspace.savePreferences(
                      workspace.preferences.copy(appearance: v),
                    );
                  }
                },
                items: ThemeMode.values
                    .map(
                      (t) => DropdownMenuItem(
                        value: t,
                        child: Text(
                          '${t.name[0].toUpperCase()}${t.name.substring(1)}',
                        ),
                      ),
                    )
                    .toList(),
              ),
            ),
          ]),
          section('Swipe actions', [
            swipe('Swipe left', p.leftSwipe, true),
            const Divider(),
            swipe('Swipe right', p.rightSwipe, false),
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 6, 16, 14),
              child: Text(
                'Swipe a message to act. Every action is also in its menu.',
                style: TextStyle(fontSize: ShepText.secondary, color: c.muted),
              ),
            ),
          ]),
          section('Message list', [
            ListTile(
              key: const ValueKey('preference-preview'),
              title: const Text('Preview lines'),
              subtitle: const Text('Sender and subject are always shown'),
              trailing: pickList<int>(
                context,
                value: p.previewLines,
                onChanged: (v) {
                  if (v != null) {
                    workspace.savePreferences(
                      workspace.preferences.copy(previewLines: v),
                    );
                  }
                },
                items: List.generate(
                  5,
                  (i) => DropdownMenuItem(value: i, child: Text('$i')),
                ),
              ),
            ),
            CheckboxListTile(
              key: const ValueKey('preference-avatars'),
              title: const Text('Sender pictures'),
              controlAffinity: ListTileControlAffinity.leading,
              value: p.avatars,
              onChanged: (v) => workspace.savePreferences(
                workspace.preferences.copy(avatars: v ?? p.avatars),
              ),
            ),
            CheckboxListTile(
              key: const ValueKey('preference-unified'),
              title: const Text('Unified inbox'),
              controlAffinity: ListTileControlAffinity.leading,
              value: p.unified,
              onChanged: (v) {
                if (v == null) return;
                workspace.savePreferences(
                  workspace.preferences.copy(unified: v),
                );
                workspace.navigate(
                  'Inbox',
                  inAccount: v ? null : workspace.accounts.firstOrNull,
                );
              },
            ),
            const SizedBox(height: 6),
          ]),
          section('Reading', [
            ListTile(
              key: const ValueKey('preference-quotes'),
              title: const Text('Quoted history'),
              trailing: pickList<String>(
                context,
                value: p.quoteMode,
                onChanged: (v) {
                  if (v != null) {
                    workspace.savePreferences(
                      workspace.preferences.copy(quoteMode: v),
                    );
                  }
                },
                items: ['Collapsed', 'Expanded', 'Latest only']
                    .map((v) => DropdownMenuItem(value: v, child: Text(v)))
                    .toList(),
              ),
            ),
            const ListTile(
              key: ValueKey('preference-images'),
              leading: ShepIcon('shield'),
              title: Text('External images blocked'),
              subtitle: Text(
                'Remote images stay blocked; inline images can still appear.',
              ),
            ),
          ]),
          section('Connections', [
            if (workspace.accountRepository != null) ...[
              for (final attempt in workspace.connectionAttempts)
                ListTile(
                  key: ValueKey('connection-${attempt.id}'),
                  leading: attempt.needsPasswords
                      ? const ShepIcon('alert-circle')
                      : const SizedBox(
                          width: 20,
                          height: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                  title: Text(attempt.account.email),
                  subtitle: Text(
                    attempt.error ??
                        (attempt.needsPasswords
                            ? 'Passwords required to continue'
                            : 'Checking connection'),
                  ),
                  trailing: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      TextButton(
                        onPressed: () =>
                            workspace.abandonConnection(attempt.id),
                        child: const Text('Cancel'),
                      ),
                      if (attempt.needsPasswords)
                        TextButton(
                          onPressed: () => Navigator.push(
                            context,
                            MaterialPageRoute<void>(
                              builder: (_) => AccountSetup(
                                workspace: workspace,
                                account: attempt.account,
                                attempt: attempt,
                              ),
                            ),
                          ),
                          child: const Text('Re-enter'),
                        ),
                    ],
                  ),
                ),
              for (final account
                  in workspace.accountRepository!.mailAccounts) ...[
                ListTile(
                  key: ValueKey('preference-account-${account.id}'),
                  leading: const ShepIcon('mail'),
                  title: Text(account.name),
                  subtitle: Text(
                    workspace.needsReconnect(account.id)
                        ? '${account.email} · Reconnect required'
                        : account.email,
                  ),
                  trailing: workspace.repository is SentPreferencesRepository
                      ? TextButton(
                          key: ValueKey('preference-sent-${account.id}'),
                          onPressed: () => Navigator.push(
                            context,
                            MaterialPageRoute<void>(
                              builder: (_) => SentPreferencesScreen(
                                workspace: workspace,
                                account: account,
                              ),
                            ),
                          ),
                          child: const Text('Sent copies'),
                        )
                      : const ShepIcon('chevron', size: 18),
                  onTap: () => Navigator.push(
                    context,
                    MaterialPageRoute<void>(
                      builder: (_) =>
                          AccountSetup(workspace: workspace, account: account),
                    ),
                  ),
                ),
                if (workspace.repository is AccountRemovalRepository)
                  ListTile(
                    key: ValueKey('preference-remove-${account.id}'),
                    leading: const ShepIcon('minus-circle'),
                    title: Text('Remove ${account.email}'),
                    onTap: () async {
                      await Navigator.push(
                        context,
                        MaterialPageRoute<void>(
                          builder: (_) => AccountRemovalScreen(
                            workspace: workspace,
                            account: account,
                          ),
                        ),
                      );
                    },
                  ),
              ],
              if (workspace.repository
                  case final AccountRemovalRepository removal)
                if (removal.pendingCredentialCleanup > 0)
                  ListTile(
                    key: const ValueKey('preference-credential-cleanup'),
                    leading: const ShepIcon('key-off'),
                    title: const Text('Saved passwords need cleanup'),
                    subtitle: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        const Text(
                          'Unlock device credential storage, then retry.',
                        ),
                        TextButton(
                          onPressed: () async {
                            await removal.cleanupCredentials();
                            await workspace.loadPage();
                          },
                          child: const Text('Retry cleanup'),
                        ),
                      ],
                    ),
                  ),
              ListTile(
                key: const ValueKey('preference-add-account'),
                leading: const ShepIcon('plus'),
                title: const Text('Add mail account'),
                onTap: () => Navigator.push(
                  context,
                  MaterialPageRoute<void>(
                    builder: (_) => AccountSetup(workspace: workspace),
                  ),
                ),
              ),
            ] else
              const ListTile(
                leading: Icon(Icons.mail_outline),
                title: Text('Mail accounts'),
                subtitle: Text(
                  'Account setup is available in the installed client.',
                ),
              ),
            if (workspace.google case final connection?)
              GoogleConnectionCard(connection: connection)
            else
              const ListTile(
                leading: ShepIcon('cloud'),
                title: Text('Google and backups'),
                subtitle: Text(
                  'Google Calendar, Drive and encrypted restore remain in the parity checklist.',
                ),
              ),
            if (workspace.supportsCalDav)
              CalDavConnectionsCard(
                key: const ValueKey('preference-caldav'),
                workspace: workspace,
                revealField: revealedTarget,
              ),
          ]),
          if (workspace.profileDiscovery case final discovery?)
            section('Profiles and sync', [
              ListTile(
                key: const ValueKey('preference-profiles'),
                leading: const ShepIcon('cloud'),
                title: const Text('Saved Google profiles'),
                subtitle: const Text('Discover account and settings profiles'),
                trailing: const ShepIcon('chevron', size: 18),
                onTap: () => Navigator.push(
                  context,
                  MaterialPageRoute<void>(
                    builder: (_) => ProfileDiscoveryScreen(
                      discovery: discovery,
                      preferences: () => workspace.preferences,
                      device: workspace.profileApplication,
                    ),
                  ),
                ),
              ),
              ProfileSyncControls(
                discovery: discovery,
                device: workspace.profileApplication,
              ),
            ]),
          const SizedBox(height: 24),
        ],
      ),
    );
  }
}
