import 'dart:async';
import 'package:flutter/material.dart';
import '../data/profile_enrollment.dart';
import '../data/profile_sync.dart';
import '../model/profile_discovery.dart';
import 'icons.dart';
import 'profile_labels.dart';
import 'theme.dart';

/// Preferences rows for ongoing preference sync: master switch, per-field
/// choices, status, Sync now and the conflict review entry point.
class ProfileSyncControls extends StatefulWidget {
  const ProfileSyncControls({
    super.key,
    required this.discovery,
    required this.device,
  });
  final ProfileDiscovery discovery;
  final ProfileEnrollmentDevice? device;
  @override
  State<ProfileSyncControls> createState() => _ProfileSyncControlsState();
}

class _ProfileSyncControlsState extends State<ProfileSyncControls> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final d = widget.discovery;
      if (mounted && d.supportsSync && d.connected && !d.syncChecked) {
        unawaited(d.refreshSync());
      }
    });
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.discovery,
    builder: (context, _) {
      final d = widget.discovery, sync = d.sync, device = widget.device;
      final c = ShepColors.of(context);
      final muted = TextStyle(fontSize: ShepText.secondary, color: c.muted);
      if (!d.supportsSync || device == null) return const SizedBox.shrink();
      if (!d.connected) {
        return Padding(
          padding: const EdgeInsets.fromLTRB(16, 6, 16, 14),
          child: Text(
            'Google Drive is not connected on this device, so preference sync is paused. Local changes are kept and no other device is signed out.',
            key: const ValueKey('profile-sync-disconnected'),
            style: muted,
          ),
        );
      }
      final error = d.syncError ?? sync?.error;
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (sync == null && !d.syncChecked)
            const ListTile(
              leading: ShepIcon('sync'),
              title: Text('Checking preference sync…'),
            ),
          if (sync == null && d.syncChecked)
            ListTile(
              leading: const ShepIcon('sync'),
              title: const Text('Preference sync is not set up'),
              subtitle: Text(
                d.enrollment?.complete == true
                    ? 'Keep this device updated with the applied profile.'
                    : 'Apply a saved profile first.',
              ),
              trailing: d.canSubscribe
                  ? TextButton(
                      key: const ValueKey('profile-sync-subscribe'),
                      onPressed: () => d.subscribeSync(device),
                      child: const Text('Keep in sync'),
                    )
                  : null,
            ),
          if (sync != null) ...[
            SwitchListTile(
              key: const ValueKey('profile-sync-master'),
              title: const Text('Sync preferences'),
              subtitle: Text(
                sync.name == null
                    ? 'With the applied profile'
                    : 'With ${sync.name}',
              ),
              value: sync.enabled,
              onChanged: d.busy
                  ? null
                  : (value) => d.configureSync(enabled: value),
            ),
            for (final field in profileSettingLabels.keys)
              SwitchListTile(
                key: ValueKey('profile-sync-field-$field'),
                dense: true,
                title: Text(profileSettingLabel(field)),
                value: sync.fieldEnabled(field),
                onChanged: d.busy || !sync.enabled
                    ? null
                    : (value) => d.configureSync(field: field, selected: value),
              ),
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 6, 16, 6),
              child: Semantics(
                key: const ValueKey('profile-sync-status'),
                liveRegion: true,
                child: Text(_status(d, sync), style: muted),
              ),
            ),
            if (error != null)
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 0, 16, 6),
                child: Text(
                  error,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              ),
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 10),
              child: Wrap(
                spacing: 8,
                children: [
                  OutlinedButton.icon(
                    key: const ValueKey('profile-sync-now'),
                    onPressed: d.busy || !sync.enabled
                        ? null
                        : () => d.syncNow(device),
                    icon: const ShepIcon('sync', size: 16),
                    label: const Text('Sync now'),
                  ),
                  if (sync.reviews > 0)
                    FilledButton.tonal(
                      key: const ValueKey('profile-sync-reviews'),
                      onPressed: d.busy
                          ? null
                          : () => Navigator.push(
                              context,
                              MaterialPageRoute<void>(
                                builder: (_) => ProfileSyncReviewScreen(
                                  discovery: d,
                                  device: device,
                                ),
                              ),
                            ),
                      child: Text(
                        'Review ${sync.reviews} preference ${sync.reviews == 1 ? 'conflict' : 'conflicts'}',
                      ),
                    ),
                ],
              ),
            ),
          ],
        ],
      );
    },
  );

  String _status(ProfileDiscovery d, ProfileSyncStatus sync) {
    if (d.syncing) return 'Syncing preferences…';
    if (!sync.enabled) return 'Paused on this device. Other devices are not affected.';
    final parts = <String>[];
    if (d.syncRan) {
      parts.add(
        'Last sync: ${d.syncApplied} applied here, ${d.syncPublished} published',
      );
    } else if (sync.last case final last?) {
      parts.add(
        'Last sync: ${last.applied} applied here, ${last.published} published',
      );
    } else {
      parts.add('Not synced yet');
    }
    if (sync.last?.remaining == true) parts.add('more to do');
    if (sync.staged > 0) parts.add('${sync.staged} unsent');
    if (sync.applications > 0) parts.add('1 waiting for this device');
    if (sync.reviews > 0) parts.add('${sync.reviews} to review');
    if (sync.unproven > 0) parts.add('${sync.unproven} unconfirmed');
    return parts.join(' · ');
  }
}

class ProfileSyncReviewScreen extends StatefulWidget {
  const ProfileSyncReviewScreen({
    super.key,
    required this.discovery,
    required this.device,
  });
  final ProfileDiscovery discovery;
  final ProfileEnrollmentDevice device;
  @override
  State<ProfileSyncReviewScreen> createState() =>
      _ProfileSyncReviewScreenState();
}

class _ProfileSyncReviewScreenState extends State<ProfileSyncReviewScreen> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted && widget.discovery.syncReviews.isEmpty) {
        unawaited(widget.discovery.refreshSync());
      }
    });
  }

  @override
  void dispose() {
    widget.discovery.closeSyncReview(notify: false);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.discovery,
    builder: (context, _) {
      final d = widget.discovery, open = d.syncReview;
      final error = d.syncError;
      return Scaffold(
        appBar: AppBar(title: const Text('Preference conflicts')),
        body: SafeArea(
          top: false,
          child: ListView(
            key: const ValueKey('profile-sync-review-list'),
            padding: const EdgeInsets.all(18),
            children: [
              const Text(
                'This device and the shared profile changed the same preference. Choose which value to keep; the choice is saved here first and then published.',
              ),
              const SizedBox(height: 12),
              if (d.busy) const LinearProgressIndicator(),
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
              if (open == null) ...[
                if (d.syncReviews.isEmpty && !d.busy)
                  const Text('No preference conflicts are waiting.'),
                for (final review in d.syncReviews)
                  Card(
                    key: ValueKey('profile-sync-review-${review.field}'),
                    child: ListTile(
                      title: Text(profileSettingLabel(review.field)),
                      subtitle: Text(
                        'This device: ${profileValueText(review.local)} · ${review.total} shared ${review.total == 1 ? 'version' : 'versions'}${review.deciding ? ' · Saving your choice' : ''}',
                      ),
                      trailing: const ShepIcon('chevron', size: 18),
                      onTap: d.busy ? null : () => d.openSyncReview(review),
                    ),
                  ),
              ] else
                ..._versions(context, d, open),
              const SizedBox(height: 24),
            ],
          ),
        ),
      );
    },
  );

  List<Widget> _versions(
    BuildContext context,
    ProfileDiscovery d,
    ProfileSyncReview open,
  ) {
    final seenAll = d.syncSeen >= open.total;
    return [
      Text(
        profileSettingLabel(open.field),
        style: Theme.of(context).textTheme.headlineSmall,
      ),
      const SizedBox(height: 8),
      Text(
        'This device: ${profileValueText(open.local)}',
        key: const ValueKey('profile-sync-local-value'),
      ),
      const SizedBox(height: 8),
      Text(
        '${d.syncSeen} of ${open.total} shared versions reviewed',
        key: const ValueKey('profile-sync-seen'),
      ),
      for (final version in d.syncVersions)
        Card(
          key: ValueKey('profile-sync-version-${version.operation}'),
          child: ListTile(
            title: Text(
              'Profile: ${profileValueText(version.reset ? null : version.value)}',
            ),
            trailing: FilledButton(
              onPressed: d.busy || !seenAll
                  ? null
                  : () => d.decideSync(
                      open,
                      widget.device,
                      shared: version.operation,
                    ),
              child: const Text('Use profile'),
            ),
          ),
        ),
      Wrap(
        spacing: 8,
        children: [
          if (d.syncVersions.length == 50)
            TextButton(
              onPressed: d.busy ? null : () => d.syncVersionsPage(first: false),
              child: const Text('Next versions'),
            ),
          if (d.syncSeen > d.syncVersions.length)
            TextButton(
              onPressed: d.busy ? null : () => d.syncVersionsPage(first: true),
              child: const Text('First versions'),
            ),
        ],
      ),
      const SizedBox(height: 12),
      if (!seenAll)
        const Text('Open every version page before choosing.'),
      OutlinedButton.icon(
        key: const ValueKey('profile-sync-keep-mine'),
        onPressed: d.busy || !seenAll
            ? null
            : () => d.decideSync(open, widget.device),
        icon: const ShepIcon('check', size: 16),
        label: const Text('Keep mine'),
      ),
      TextButton(
        onPressed: d.busy ? null : d.closeSyncReview,
        child: const Text('Back to conflicts'),
      ),
    ];
  }
}
