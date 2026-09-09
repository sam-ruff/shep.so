import 'package:flutter/material.dart';
import '../model/profile_discovery.dart';
import 'google_connection.dart';

String _count(int n, String noun) => '$n $noun${n == 1 ? '' : 's'}';

class ProfileDiscoveryScreen extends StatelessWidget {
  const ProfileDiscoveryScreen({super.key, required this.discovery});
  final ProfileDiscovery discovery;
  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: discovery,
    builder: (context, _) {
      final d = discovery, state = d.state;
      final colors = Theme.of(context).colorScheme;
      return Scaffold(
        appBar: AppBar(title: const Text('Profiles and sync')),
        body: ListView(
          key: const ValueKey('profile-discovery-list'),
          padding: const EdgeInsets.all(18),
          children: [
            Card(child: GoogleConnectionCard(connection: d.google)),
            const SizedBox(height: 16),
            Text(
              'Saved profiles',
              style: Theme.of(context).textTheme.titleMedium,
            ),
            const SizedBox(height: 8),
            const Text(
              'Find account and settings profiles saved with Google. Applying a profile and continuous sync are not available in this build yet.',
            ),
            if (!d.configured)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 12),
                child: Text(
                  'Profile discovery is not configured in this build. Install a build configured for Google profiles.',
                ),
              ),
            if (!d.connected)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 12),
                child: Text('Sign in and enable Drive to find saved profiles.'),
              ),
            const SizedBox(height: 12),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                FilledButton.icon(
                  key: const ValueKey('discover-profiles'),
                  onPressed: !d.busy && d.connected && d.configured
                      ? () => d.discover()
                      : null,
                  icon: const Icon(Icons.cloud_sync_outlined),
                  label: Text(
                    d.error != null
                        ? 'Retry discovery'
                        : state == null
                        ? 'Find profiles'
                        : state.complete
                        ? 'Refresh profiles'
                        : 'Resume discovery',
                  ),
                ),
                if (d.busy)
                  OutlinedButton(
                    onPressed: d.paused ? null : d.pause,
                    child: Text(d.paused ? 'Pausing…' : 'Pause discovery'),
                  ),
                if (state != null && !d.busy)
                  TextButton(
                    onPressed: () => d.discover(full: true),
                    child: const Text('Rescan all profiles'),
                  ),
              ],
            ),
            if (d.busy)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 14),
                child: LinearProgressIndicator(),
              ),
            if (d.error case final error?)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 12),
                child: Semantics(
                  liveRegion: true,
                  child: Text(error, style: TextStyle(color: colors.error)),
                ),
              ),
            if (state != null) ...[
              const SizedBox(height: 12),
              Text(
                d.busy
                    ? 'Checking saved profiles…'
                    : state.complete
                    ? 'Profile discovery complete'
                    : d.paused
                    ? 'Discovery paused'
                    : 'Discovery is incomplete',
                style: Theme.of(context).textTheme.titleSmall,
              ),
              Text(
                '${_count(state.profiles, 'saved profile')} · ${_count(state.files, 'profile file')} checked',
              ),
              if (!state.complete)
                const Text(
                  'Saved results may be incomplete. Resume or retry before choosing a profile.',
                ),
              if (state.incompleteProfiles > 0)
                Text(
                  '${_count(state.incompleteProfiles, 'profile')} waiting for missing history.',
                  style: TextStyle(color: colors.error),
                ),
              const SizedBox(height: 12),
              if (d.profiles.isEmpty && state.complete)
                Text(
                  d.after == null
                      ? 'No continuous profiles found. Legacy backups are separate from these profiles.'
                      : 'No more profiles on this page.',
                ),
              for (final profile in d.profiles)
                Card(
                  key: ValueKey('discovered-${profile.cursor}'),
                  child: ListTile(
                    leading: Icon(
                      profile.removed
                          ? Icons.delete_outline
                          : Icons.account_circle_outlined,
                    ),
                    title: Text(
                      profile.nameConflict
                          ? 'Profile name needs review'
                          : profile.name ?? 'Unnamed profile',
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                    ),
                    subtitle: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          '${_count(profile.accounts, 'account')} · ${_count(profile.settings, 'setting')}',
                        ),
                        if (profile.removed) const Text('Removed profile'),
                        if (profile.waiting > 0 || profile.ready > 0)
                          const Text('History is incomplete'),
                        if (profile.conflicts > 0)
                          Text(
                            '${_count(profile.conflicts, 'conflict')} ${profile.conflicts == 1 ? 'needs' : 'need'} review',
                            style: TextStyle(color: colors.error),
                          ),
                      ],
                    ),
                    isThreeLine:
                        profile.removed ||
                        profile.waiting > 0 ||
                        profile.ready > 0 ||
                        profile.conflicts > 0,
                  ),
                ),
              Wrap(
                spacing: 8,
                children: [
                  if (d.after != null)
                    TextButton(
                      onPressed: d.canPage ? () => d.page(first: true) : null,
                      child: const Text('First page'),
                    ),
                  if (d.profiles.length == 50)
                    TextButton(
                      onPressed: d.canPage ? () => d.page(first: false) : null,
                      child: const Text('Next profiles'),
                    ),
                ],
              ),
            ],
            const SizedBox(height: 24),
          ],
        ),
      );
    },
  );
}
