import 'dart:async';
import 'package:flutter/material.dart';
import '../data/profile_discovery.dart';
import '../data/profile_enrollment.dart';
import '../model/profile_discovery.dart';
import 'icons.dart';

const _labels = {
  'appearance': 'Appearance',
  'left_swipe': 'Swipe left',
  'right_swipe': 'Swipe right',
  'preview_lines': 'Preview lines',
  'sender_pictures': 'Sender pictures',
  'unified_inbox': 'Unified inbox',
  'reply_display': 'Quoted history',
  'tooltips': 'Tooltips',
};
String _value(Object? value) => value == null
    ? 'Reset to default'
    : value is bool
    ? (value ? 'On' : 'Off')
    : '$value';

class ProfileEnrollmentScreen extends StatefulWidget {
  const ProfileEnrollmentScreen({
    super.key,
    required this.discovery,
    required this.device,
    this.profile,
  });
  final ProfileDiscovery discovery;
  final ProfileEnrollmentDevice device;
  final DiscoveredProfile? profile;
  @override
  State<ProfileEnrollmentScreen> createState() =>
      _ProfileEnrollmentScreenState();
}

class _ProfileEnrollmentScreenState extends State<ProfileEnrollmentScreen> {
  bool accounts = true, settings = true;
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      if (widget.profile case final profile?) {
        unawaited(widget.discovery.prepareEnrollment(profile, widget.device));
      } else if (widget.discovery.enrollment?.needsReview == true) {
        unawaited(widget.discovery.enrollmentPage(first: true));
      }
    });
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.discovery,
    builder: (context, _) {
      final d = widget.discovery, review = d.enrollment;
      final error = d.error ?? review?.error;
      return Scaffold(
        appBar: AppBar(title: const Text('Use profile on this device')),
        body: SafeArea(
          top: false,
          child: ListView(
            key: const ValueKey('profile-enrollment-list'),
            padding: const EdgeInsets.all(18),
            children: [
              Text(
                review?.name ?? widget.profile?.name ?? 'Profile review',
                style: Theme.of(context).textTheme.headlineSmall,
              ),
              const SizedBox(height: 12),
              const Text(
                'Review accounts and preferences before applying them. Imported accounts need their passwords in Connections. Existing mail and drafts stay on this device.',
              ),
              const SizedBox(height: 12),
              if (!d.connected)
                const Text(
                  'Reconnect the same Google account to resume this enrollment.',
                ),
              if (review != null && !review.needsReview && !review.complete)
                Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: Text(
                    review.preparing
                        ? '${review.copied} of ${review.total} records prepared for review'
                        : 'Connections applied: ${review.applied} · Kept: ${review.kept}',
                  ),
                ),
              if (d.busy) ...[
                const LinearProgressIndicator(),
                const SizedBox(height: 12),
                Text(
                  review?.preparing == true
                      ? 'Preparing review · ${review!.copied} of ${review.total} records'
                      : 'Saving reviewed changes…',
                ),
                Align(
                  alignment: Alignment.centerLeft,
                  child: OutlinedButton(
                    onPressed: d.paused ? null : d.pause,
                    child: Text(d.paused ? 'Pausing…' : 'Pause enrollment'),
                  ),
                ),
              ],
              if (error != null)
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
              if (review?.needsReview == true) ...[
                CheckboxListTile(
                  controlAffinity: ListTileControlAffinity.leading,
                  contentPadding: EdgeInsets.zero,
                  title: const Text('Account connections'),
                  value: accounts,
                  onChanged: d.busy
                      ? null
                      : (v) => setState(() => accounts = v ?? accounts),
                ),
                CheckboxListTile(
                  controlAffinity: ListTileControlAffinity.leading,
                  contentPadding: EdgeInsets.zero,
                  title: const Text('Appearance and reading preferences'),
                  value: settings,
                  onChanged: d.busy
                      ? null
                      : (v) => setState(() => settings = v ?? settings),
                ),
                const Text(
                  'Newer local preference edits are kept. Unselected fields stay as they are. Unsupported fields and conflicts remain in the shared profile for later review.',
                ),
                const SizedBox(height: 12),
                Text(
                  '${review!.rows} review items',
                  style: Theme.of(context).textTheme.titleMedium,
                ),
                for (final row in d.enrollmentRows)
                  Card(
                    key: ValueKey('enrollment-row-${row.position}'),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        CheckboxListTile(
                          title: Text(
                            row.account?.name ??
                                _labels[row.target.replaceFirst(
                                  'setting:',
                                  '',
                                )] ??
                                row.target,
                          ),
                          subtitle: Text(
                            row.account?.email ??
                                'Profile: ${_value(row.value)} · This device: ${_value(review.baselineValues[row.target.replaceFirst('setting:', '')])}',
                          ),
                          value: row.selected,
                          onChanged:
                              !d.busy &&
                                  row.available &&
                                  (row.kind == 'account' ? accounts : settings)
                              ? (v) => d.chooseEnrollment(row, v ?? false)
                              : null,
                          controlAffinity: ListTileControlAffinity.leading,
                        ),
                        if (row.reason case final reason?)
                          Padding(
                            padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
                            child: Text(reason),
                          ),
                        if (row.account != null)
                          Align(
                            alignment: Alignment.centerLeft,
                            child: TextButton(
                              onPressed: () => _details(context, row),
                              child: const Text('Connection details'),
                            ),
                          ),
                      ],
                    ),
                  ),
                Wrap(
                  spacing: 8,
                  children: [
                    if (d.enrollmentAfter > 0)
                      TextButton(
                        onPressed: d.busy
                            ? null
                            : () => d.enrollmentPage(first: true),
                        child: const Text('First review page'),
                      ),
                    if (d.enrollmentRows.length == 50)
                      TextButton(
                        onPressed: d.busy
                            ? null
                            : () => d.enrollmentPage(first: false),
                        child: const Text('Next review items'),
                      ),
                  ],
                ),
                const SizedBox(height: 12),
                FilledButton.icon(
                  key: const ValueKey('apply-profile'),
                  onPressed: d.busy || !d.connected
                      ? null
                      : () => d.approveEnrollment(
                          widget.device,
                          accounts: accounts,
                          settings: settings,
                        ),
                  icon: const ShepIcon('check'),
                  label: const Text('Apply selected changes'),
                ),
              ],
              if (review != null &&
                  !review.needsReview &&
                  !d.busy &&
                  !review.complete)
                FilledButton(
                  onPressed: d.connected
                      ? () => d.resumeEnrollment(widget.device)
                      : null,
                  child: const Text('Resume enrollment'),
                ),
              if (review != null && (review.needsReview || review.preparing))
                TextButton(
                  onPressed: d.busy || !d.connected
                      ? null
                      : () async {
                          await d.cancelEnrollment();
                          if (context.mounted && d.enrollment == null) {
                            Navigator.pop(context);
                          }
                        },
                  child: const Text('Cancel review'),
                ),
              if (review?.complete == true) ...[
                const ShepIcon('check-circle', size: 40),
                const SizedBox(height: 12),
                Semantics(
                  key: const ValueKey('profile-enrollment-summary'),
                  container: true,
                  liveRegion: true,
                  label:
                      'Profile applied · ${review!.applied} account connections applied, ${review.kept} kept.',
                  child: ExcludeSemantics(
                    child: Text(
                      'Profile applied · ${review.applied} account connections applied, ${review.kept} kept.',
                    ),
                  ),
                ),
                Text(
                  '${(review.settingsReceipt?['applied'] as List? ?? []).length} preferences applied · ${(review.settingsReceipt?['kept'] as List? ?? []).length} newer local preferences kept.',
                ),
                if (error != null)
                  TextButton(
                    onPressed: d.busy
                        ? null
                        : () => d.resumeEnrollment(widget.device),
                    child: const Text('Retry account refresh'),
                  ),
                const Text(
                  'Open Connections to reconnect imported accounts. Continuous background profile sync is still being built.',
                ),
                const SizedBox(height: 12),
                FilledButton(
                  onPressed: () => Navigator.pop(context),
                  child: const Text('Done'),
                ),
              ],
              const SizedBox(height: 24),
            ],
          ),
        ),
      );
    },
  );
}

void _details(BuildContext context, EnrollmentRow row) {
  final account = row.account!;
  showDialog<void>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(account.name),
      content: SingleChildScrollView(
        child: SelectionArea(
          child: Text(
            [
              account.email,
              'Incoming: ${account.protocol} · ${account.host}:${account.port} · ${account.security}',
              'Username: ${account.username} · ${account.authentication}',
              'SMTP: ${account.smtpHost}:${account.smtpPort} · ${account.smtpSecurity}',
              'SMTP username: ${account.smtpUsername} · ${account.smtpAuthentication}',
              'Separate SMTP password: ${account.separatePassword ? 'Yes' : 'No'}',
              'Sent copies: ${account.sentCopy}${account.sentFolder.isEmpty ? '' : ' · ${account.sentFolder}'}',
              if (row.local case final local?)
                'Current incoming: ${local.host}:${local.port} · ${local.username}',
              'Passwords are entered separately on this device.',
            ].join('\n\n'),
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Close'),
        ),
      ],
    ),
  );
}
