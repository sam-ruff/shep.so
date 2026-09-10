import 'package:flutter/material.dart';
import '../data/profile_creation.dart';
import '../model/preferences.dart';
import '../model/profile_discovery.dart';

class ProfileCreationScreen extends StatefulWidget {
  const ProfileCreationScreen({
    super.key,
    required this.discovery,
    required this.preferences,
    this.newProfile = false,
  });
  final ProfileDiscovery discovery;
  final Preferences Function() preferences;
  final bool newProfile;
  @override
  State<ProfileCreationScreen> createState() => _ProfileCreationScreenState();
}

class _ProfileCreationScreenState extends State<ProfileCreationScreen> {
  final name = TextEditingController(text: 'My profile');
  bool accounts = true, settings = true;
  late bool creating = widget.newProfile;
  String? formError;
  @override
  void dispose() {
    name.dispose();
    super.dispose();
  }

  Future<void> prepare() async {
    if (name.text.trim().isEmpty) {
      setState(() => formError = 'Give this profile a name.');
      return;
    }
    setState(() => formError = null);
    await widget.discovery.prepareCreation(
      name.text,
      accounts: accounts,
      settings: settings ? widget.preferences().profileSettings() : {},
    );
    if (mounted && widget.discovery.creation?.needsReview == true) {
      setState(() => creating = false);
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.discovery,
    builder: (context, _) {
      final d = widget.discovery, c = d.creation;
      final form = c == null || creating && c.complete;
      return Scaffold(
        appBar: AppBar(
          title: Text(form ? 'Create a profile' : 'Profile publication'),
        ),
        body: SafeArea(
          top: false,
          child: ListView(
            key: const ValueKey('profile-creation-list'),
            padding: const EdgeInsets.all(18),
            children: [
              const Text(
                'Save account definitions and app settings in your private Google storage. Mail and passwords stay on this device.',
              ),
              const SizedBox(height: 12),
              if (form) ...[
                TextField(
                  key: const ValueKey('profile-name'),
                  controller: name,
                  enabled: !d.busy,
                  maxLength: 256,
                  decoration: const InputDecoration(labelText: 'Profile name'),
                ),
                SwitchListTile(
                  title: const Text('Include mail accounts'),
                  subtitle: const Text(
                    'All saved account definitions; reconnect on another device.',
                  ),
                  value: accounts,
                  onChanged: d.busy
                      ? null
                      : (v) => setState(() => accounts = v),
                ),
                SwitchListTile(
                  title: const Text('Include app settings'),
                  subtitle: const Text(
                    'Appearance, swipes, previews and reading preferences.',
                  ),
                  value: settings,
                  onChanged: d.busy
                      ? null
                      : (v) => setState(() => settings = v),
                ),
                const SizedBox(height: 12),
                FilledButton.icon(
                  key: const ValueKey('prepare-profile'),
                  onPressed: d.canCreate ? prepare : null,
                  icon: const Icon(Icons.preview_outlined),
                  label: const Text('Review profile'),
                ),
                const SizedBox(height: 12),
                const Text(
                  'This creates a separate profile. Existing profiles and legacy backups are kept.',
                ),
              ] else ...[
                Text(c.name, style: Theme.of(context).textTheme.titleLarge),
                const SizedBox(height: 8),
                Text(
                  '${c.accounts} ${c.accounts == 1 ? 'account' : 'accounts'} · ${c.settings} ${c.settings == 1 ? 'setting' : 'settings'}',
                ),
                const SizedBox(height: 12),
                if (c.settingValues.isNotEmpty)
                  Card(
                    child: Column(
                      children: [
                        for (final entry in c.settingValues.entries)
                          ListTile(
                            dense: true,
                            title: Text(_settingLabel(entry.key)),
                            trailing: Text(_settingValue(entry.value)),
                          ),
                      ],
                    ),
                  ),
                if (c.needsReview) ...[
                  const Text(
                    'Review the frozen account list before publishing. Later local edits do not change this review.',
                  ),
                  const SizedBox(height: 12),
                  for (final row in d.creationAccounts) _AccountReview(row),
                  Wrap(
                    spacing: 8,
                    children: [
                      if (d.creationAfter > 0)
                        TextButton(
                          onPressed: d.busy
                              ? null
                              : () => d.reviewCreationAccounts(first: true),
                          child: const Text('First accounts'),
                        ),
                      if (d.creationAccounts.length == 50)
                        TextButton(
                          onPressed: d.busy
                              ? null
                              : () => d.reviewCreationAccounts(first: false),
                          child: const Text('Next accounts'),
                        ),
                    ],
                  ),
                  const SizedBox(height: 12),
                  Wrap(
                    spacing: 8,
                    runSpacing: 8,
                    children: [
                      FilledButton.icon(
                        key: const ValueKey('publish-profile'),
                        onPressed: d.canCreate
                            ? () => d.approveCreation(
                                c.settings == 0
                                    ? {}
                                    : widget.preferences().profileSettings(),
                              )
                            : null,
                        icon: const Icon(Icons.cloud_upload_outlined),
                        label: const Text('Publish profile'),
                      ),
                      TextButton(
                        onPressed: d.busy
                            ? null
                            : () async {
                                await d.cancelCreation();
                                if (mounted && d.creation == null) {
                                  setState(() => creating = true);
                                }
                              },
                        child: const Text('Cancel review'),
                      ),
                    ],
                  ),
                ] else if (c.complete) ...[
                  const Icon(Icons.cloud_done_outlined, size: 36),
                  const SizedBox(height: 8),
                  const Text('Profile saved to Google'),
                  const SizedBox(height: 8),
                  Text('${c.uploaded} of ${c.total} profile files confirmed.'),
                ] else ...[
                  Text(
                    c.phase == 'staging'
                        ? 'Preparing the saved profile'
                        : 'Publishing the saved profile',
                  ),
                  const SizedBox(height: 12),
                  LinearProgressIndicator(
                    value: c.total == 0
                        ? null
                        : (c.phase == 'staging' ? c.staged : c.uploaded) /
                              c.total,
                  ),
                  const SizedBox(height: 8),
                  Text(
                    c.phase == 'staging'
                        ? '${c.staged} of ${c.total} files prepared'
                        : '${c.uploaded} of ${c.total} files confirmed by Google',
                  ),
                  const SizedBox(height: 12),
                  if (d.publishing && d.busy)
                    OutlinedButton(
                      onPressed: d.paused ? null : d.pause,
                      child: Text(d.paused ? 'Pausing…' : 'Pause publication'),
                    )
                  else
                    FilledButton.icon(
                      onPressed: d.busy || !d.connected
                          ? null
                          : d.resumeCreation,
                      icon: const Icon(Icons.cloud_sync_outlined),
                      label: const Text('Resume publication'),
                    ),
                  const SizedBox(height: 8),
                  const Text(
                    'You can keep using mail. Pausing keeps this publication and lets its current step finish.',
                  ),
                ],
              ],
              if (formError ?? d.error ?? c?.error case final error?)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 12),
                  child: Semantics(
                    liveRegion: true,
                    child: Text(
                      error,
                      style: TextStyle(
                        color: Theme.of(context).colorScheme.error,
                      ),
                    ),
                  ),
                ),
              if (d.busy && !d.publishing)
                const Padding(
                  padding: EdgeInsets.symmetric(vertical: 12),
                  child: LinearProgressIndicator(),
                ),
              const SizedBox(height: 16),
              const Text(
                'Open a saved profile in Profiles and sync to apply its reviewed accounts and preferences. Continuous background sync is not available yet.',
              ),
              const SizedBox(height: 24),
            ],
          ),
        ),
      );
    },
  );
}

class _AccountReview extends StatelessWidget {
  const _AccountReview(this.row);
  final ProfileAccountReview row;
  @override
  Widget build(BuildContext context) {
    final a = row.account;
    return Card(
      child: ListTile(
        title: Text(a.name, maxLines: 2, overflow: TextOverflow.ellipsis),
        subtitle: Text(
          '${a.email}\n${a.host}:${a.port}',
          maxLines: 3,
          overflow: TextOverflow.ellipsis,
        ),
        isThreeLine: true,
        trailing: const Icon(Icons.info_outline),
        onTap: () => showDialog<void>(
          context: context,
          builder: (context) => AlertDialog(
            title: Text(a.name),
            scrollable: true,
            content: SelectionArea(
              child: Text(
                'Email: ${a.email}\n\n'
                'Incoming: ${a.protocol.toUpperCase()}\n'
                'Server: ${a.host}:${a.port}\n'
                'Security: ${a.security == 'Tls' ? 'TLS' : 'STARTTLS'}\n'
                'Username: ${a.username}\n\n'
                'Outgoing: SMTP\n'
                'Server: ${a.smtpHost}:${a.smtpPort}\n'
                'Security: ${a.smtpSecurity == 'Tls' ? 'TLS' : 'STARTTLS'}\n'
                'Username: ${a.smtpUsername.isEmpty ? a.username : a.smtpUsername}\n'
                'Authentication: ${a.smtpAuthentication}\n\n'
                'Passwords are not included.',
              ),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(context),
                child: const Text('Close'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

String _settingLabel(String key) =>
    const {
      'appearance': 'Appearance',
      'left_swipe': 'Swipe left',
      'right_swipe': 'Swipe right',
      'preview_lines': 'Preview lines',
      'sender_pictures': 'Sender pictures',
      'unified_inbox': 'Unified inbox',
      'reply_display': 'Quoted history',
      'tooltips': 'Tooltips',
      'image_policy': 'External images',
      'cross_account_moves': 'Cross-account moves',
      'group_conversations': 'Group conversations',
      'desktop_badges': 'Unread badges',
    }[key] ??
    'Additional setting';
String _settingValue(Object? value) {
  if (value is bool) return value ? 'On' : 'Off';
  final text = value.toString();
  return const {
        'LatestOnly': 'Latest only',
        'read': 'Read / unread',
        'star': 'Flag / unflag',
        'BlockAll': 'Blocked',
        'AllowAll': 'Allowed',
      }[text] ??
      (text.isEmpty ? '' : text[0].toUpperCase() + text.substring(1));
}
