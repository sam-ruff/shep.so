import 'package:flutter/material.dart';
import '../model/profile_discovery.dart';
import 'google_connection.dart';
import 'profile_creation.dart';
import 'profile_enrollment.dart';
import '../data/profile_enrollment.dart';
import '../model/preferences.dart';
import 'icons.dart';

String _count(int n, String noun) => '$n $noun${n == 1 ? '' : 's'}';

class ProfileDiscoveryScreen extends StatelessWidget {
  const ProfileDiscoveryScreen({
    super.key,
    required this.discovery,
    this.preferences,
    this.device,
  });
  final ProfileDiscovery discovery;
  final ProfileEnrollmentDevice? device;
  final Preferences Function()? preferences;
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
              'Find and review account and settings profiles saved with Google. Continuous background profile sync is still being built.',
            ),
            if (d.supportsEnrollment && device != null && d.enrollment != null)
              Card(
                child: ListTile(
                  title: Text(d.enrollment!.name ?? 'Saved enrollment'),
                  subtitle: Text(
                    d.enrollment!.complete
                        ? 'Profile applied on this device'
                        : 'Resume profile review and application',
                  ),
                  trailing: const ShepIcon('chevron', size: 18),
                  onTap: () => Navigator.push(
                    context,
                    MaterialPageRoute<void>(
                      builder: (_) => ProfileEnrollmentScreen(
                        discovery: d,
                        device: device!,
                      ),
                    ),
                  ),
                ),
              ),
            if (d.supportsCreation && preferences != null) ...[
              const SizedBox(height: 12),
              if (d.creation case final creation?)
                Card(
                  child: ListTile(
                    title: Text(creation.name),
                    subtitle: Text(
                      creation.complete
                          ? 'Profile saved to Google'
                          : creation.needsReview
                          ? 'Profile review saved on this device'
                          : 'Profile publication needs to finish',
                    ),
                    trailing: const ShepIcon('chevron', size: 18),
                    onTap: () => Navigator.push(
                      context,
                      MaterialPageRoute<void>(
                        builder: (_) => ProfileCreationScreen(
                          discovery: d,
                          preferences: preferences!,
                        ),
                      ),
                    ),
                  ),
                ),
              OutlinedButton.icon(
                onPressed:
                    d.canCreate && (d.creation == null || d.creation!.complete)
                    ? () => Navigator.push(
                        context,
                        MaterialPageRoute<void>(
                          builder: (_) => ProfileCreationScreen(
                            discovery: d,
                            preferences: preferences!,
                            newProfile: true,
                          ),
                        ),
                      )
                    : null,
                icon: const ShepIcon('plus'),
                label: const Text('Create profile'),
              ),
            ],
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
                  icon: const ShepIcon('cloud'),
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
                if (d.busy && !d.publishing && !d.enrolling)
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
                    onTap:
                        device != null &&
                            d.canEnroll &&
                            !profile.removed &&
                            profile.initialized &&
                            profile.waiting == 0 &&
                            profile.ready == 0 &&
                            (d.enrollment == null || d.enrollment!.complete)
                        ? () => Navigator.push(
                            context,
                            MaterialPageRoute<void>(
                              builder: (_) => ProfileEnrollmentScreen(
                                discovery: d,
                                device: device!,
                                profile: profile,
                              ),
                            ),
                          )
                        : null,
                    leading: ShepIcon(profile.removed ? 'trash' : 'user'),
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
                        if (!profile.removed && !profile.initialized)
                          const Text('Setup is not complete'),
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
                        !profile.initialized ||
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
