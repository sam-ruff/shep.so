import 'account_removal.dart';
import 'package:flutter/material.dart';
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

class PreferencesView extends StatelessWidget {
  const PreferencesView({super.key, required this.workspace});
  final Workspace workspace;
  @override
  Widget build(BuildContext context) {
    final p = workspace.preferences;
    final c = ShepColors.of(context);
    Widget section(String title, List<Widget> children) => Padding(
      padding: const EdgeInsets.only(bottom: 18),
      child: SettingsCard(title: title, children: children),
    );
    Widget swipe(String label, MailAction value, bool left) => ListTile(
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
    return ListView(
      key: const ValueKey('preferences-list'),
      padding: const EdgeInsets.all(18),
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(2, 0, 2, 18),
          child: Text(
            'Make Shep feel like home.',
            style: TextStyle(fontSize: ShepText.secondary, color: c.muted),
          ),
        ),
        section('Appearance', [
          ListTile(
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
            title: const Text('Sender pictures'),
            controlAffinity: ListTileControlAffinity.leading,
            value: p.avatars,
            onChanged: (v) => workspace.savePreferences(
              workspace.preferences.copy(avatars: v ?? p.avatars),
            ),
          ),
          CheckboxListTile(
            title: const Text('Unified inbox'),
            controlAffinity: ListTileControlAffinity.leading,
            value: p.unified,
            onChanged: (v) {
              if (v == null) return;
              workspace.savePreferences(workspace.preferences.copy(unified: v));
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
              items: [
                'Collapsed',
                'Expanded',
                'Latest only',
              ].map((v) => DropdownMenuItem(value: v, child: Text(v))).toList(),
            ),
          ),
          const ListTile(
            leading: ShepIcon('shield'),
            title: Text('External images blocked'),
            subtitle: Text('Messages are displayed as selectable text.'),
          ),
        ]),
        section('Connections', [
          if (workspace.accountRepository != null) ...[
            for (final account
                in workspace.accountRepository!.mailAccounts) ...[
              ListTile(
                leading: const ShepIcon('mail'),
                title: Text(account.name),
                subtitle: Text(
                  workspace.needsReconnect(account.id)
                      ? '${account.email} · Reconnect required'
                      : account.email,
                ),
                trailing: workspace.repository is SentPreferencesRepository
                    ? TextButton(
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
        ]),
        if (workspace.profileDiscovery case final discovery?)
          section('Profiles and sync', [
            ListTile(
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
          ]),
        if (workspace.savingPreferences) const Text('Saving preferences…'),
        const SizedBox(height: 24),
      ],
    );
  }
}
