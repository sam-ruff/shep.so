import 'account_removal.dart';
import 'package:flutter/material.dart';
import '../model/mail.dart';
import '../model/workspace.dart';
import 'theme.dart';
import 'account_setup.dart';
import 'sent_preferences.dart';
import '../data/accounts.dart';

class PreferencesView extends StatelessWidget {
  const PreferencesView({super.key, required this.workspace});
  final Workspace workspace;
  @override
  Widget build(BuildContext context) {
    final p = workspace.preferences;
    Widget section(String title, List<Widget> children) => Padding(
      padding: const EdgeInsets.only(bottom: 20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 4, 0, 10),
            child: Text(title, style: Theme.of(context).textTheme.titleSmall),
          ),
          Card(
            margin: EdgeInsets.zero,
            child: Column(children: children),
          ),
        ],
      ),
    );
    Widget swipe(String label, MailAction value, bool left) => ListTile(
      title: Text(label),
      leading: Icon(
        left ? Icons.swipe_left_outlined : Icons.swipe_right_outlined,
      ),
      trailing: DropdownButton<MailAction>(
        value: value,
        underline: const SizedBox(),
        onChanged: (v) {
          if (v != null) {
            workspace.savePreferences(
              left ? p.copy(leftSwipe: v) : p.copy(rightSwipe: v),
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
                    Icon(actionIcon(a.name), size: 18),
                    const SizedBox(width: 8),
                    Text(a.label, style: const TextStyle(fontSize: 12)),
                  ],
                ),
              ),
            )
            .toList(),
      ),
    );
    return ListView(
      padding: const EdgeInsets.all(18),
      children: [
        section('Appearance', [
          ListTile(
            title: const Text('Theme'),
            leading: const Icon(Icons.contrast),
            trailing: DropdownButton<ThemeMode>(
              value: p.appearance,
              underline: const SizedBox(),
              onChanged: (v) {
                if (v != null) workspace.savePreferences(p.copy(appearance: v));
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
          const Padding(
            padding: EdgeInsets.fromLTRB(16, 2, 16, 16),
            child: Text(
              'Swipe a message to act. Every action is also in its menu.',
              style: TextStyle(fontSize: 12),
            ),
          ),
        ]),
        section('Message list', [
          ListTile(
            title: const Text('Preview lines'),
            subtitle: const Text('Sender and subject are always shown'),
            trailing: DropdownButton<int>(
              value: p.previewLines,
              underline: const SizedBox(),
              onChanged: (v) {
                if (v != null) {
                  workspace.savePreferences(p.copy(previewLines: v));
                }
              },
              items: List.generate(
                5,
                (i) => DropdownMenuItem(value: i, child: Text('$i')),
              ),
            ),
          ),
          SwitchListTile(
            title: const Text('Sender pictures'),
            value: p.avatars,
            onChanged: (v) => workspace.savePreferences(p.copy(avatars: v)),
          ),
          SwitchListTile(
            title: const Text('Unified inbox'),
            value: p.unified,
            onChanged: (v) {
              workspace.savePreferences(p.copy(unified: v));
              workspace.navigate(
                'Inbox',
                inAccount: v ? null : workspace.accounts.firstOrNull,
              );
            },
          ),
        ]),
        section('Reading', [
          ListTile(
            title: const Text('Quoted history'),
            trailing: DropdownButton<String>(
              value: p.quoteMode,
              underline: const SizedBox(),
              onChanged: (v) {
                if (v != null) workspace.savePreferences(p.copy(quoteMode: v));
              },
              items: [
                'Collapsed',
                'Expanded',
                'Latest only',
              ].map((v) => DropdownMenuItem(value: v, child: Text(v))).toList(),
            ),
          ),
          const ListTile(
            leading: Icon(Icons.shield_outlined),
            title: Text('External images blocked'),
            subtitle: Text('Messages are displayed as selectable text.'),
          ),
        ]),
        section('Connections', [
          if (workspace.accountRepository != null) ...[
            for (final account
                in workspace.accountRepository!.mailAccounts) ...[
              ListTile(
                leading: const Icon(Icons.mail_outline),
                title: Text(account.name),
                subtitle: Text(account.email),
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
                    : const Icon(Icons.chevron_right),
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
                  leading: const Icon(Icons.remove_circle_outline),
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
                  leading: const Icon(Icons.key_off_outlined),
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
              leading: const Icon(Icons.add),
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
          const ListTile(
            leading: Icon(Icons.cloud_outlined),
            title: Text('Google and backups'),
            subtitle: Text(
              'Google Calendar, Drive and encrypted restore remain in the parity checklist.',
            ),
          ),
        ]),
        if (workspace.savingPreferences) const Text('Saving preferences…'),
        const SizedBox(height: 24),
      ],
    );
  }
}
