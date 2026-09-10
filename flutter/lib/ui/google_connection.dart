import 'package:flutter/material.dart';
import '../model/google_connection.dart';

class GoogleConnectionCard extends StatelessWidget {
  const GoogleConnectionCard({super.key, required this.connection});
  final GoogleConnection connection;
  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: connection,
    builder: (context, _) {
      final c = connection, requested = c.requested, active = c.active;
      final editable = c.loaded && !c.committing;
      return Semantics(
        container: true,
        explicitChildNodes: true,
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Google connection',
                style: Theme.of(context).textTheme.titleSmall,
              ),
              const SizedBox(height: 8),
              const Text(
                'Choose permissions for this device. Calendar sync, backups and shared profiles are still being connected.',
                style: TextStyle(fontSize: 12),
              ),
              if (active != null) ...[
                const SizedBox(height: 12),
                Text(active.email, key: const ValueKey('google-active-email')),
                Text(
                  'Saved access: ${_permissions(active.permissions)}',
                  key: const ValueKey('google-active-permissions'),
                  style: const TextStyle(fontSize: 12),
                ),
              ],
              const SizedBox(height: 16),
              const Text(
                'Permissions for the next sign-in',
                style: TextStyle(fontSize: 12),
              ),
              SwitchListTile(
                key: const ValueKey('google-drive'),
                contentPadding: EdgeInsets.zero,
                title: const Text('Private Drive storage'),
                subtitle: const Text('Only Shep’s app data'),
                value: requested.drive,
                onChanged: editable
                    ? (value) => c.choose(
                        GooglePermissions(
                          drive: value,
                          calendar: requested.calendar,
                        ),
                      )
                    : null,
              ),
              DropdownButtonFormField<GoogleCalendarPermission>(
                isExpanded: true,
                style: Theme.of(context).textTheme.bodySmall,
                key: ValueKey('google-calendar-${requested.calendar.name}'),
                initialValue: requested.calendar,
                decoration: const InputDecoration(labelText: 'Calendar access'),
                items: GoogleCalendarPermission.values
                    .map(
                      (value) => DropdownMenuItem(
                        value: value,
                        child: Text(switch (value) {
                          GoogleCalendarPermission.off => 'No Calendar access',
                          GoogleCalendarPermission.read => 'Read calendars',
                          GoogleCalendarPermission.edit =>
                            'Read and edit calendars',
                        }),
                      ),
                    )
                    .toList(),
                onChanged: editable
                    ? (value) {
                        if (value != null) {
                          c.choose(
                            GooglePermissions(
                              drive: requested.drive,
                              calendar: value,
                            ),
                          );
                        }
                      }
                    : null,
              ),
              const SizedBox(height: 12),
              const Text(
                'Changing these choices does not change your saved access until sign-in succeeds.',
                style: TextStyle(fontSize: 12),
              ),
              const SizedBox(height: 12),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  FilledButton(
                    onPressed: c.loaded && !c.busy && !c.cleanupPending
                        ? c.connect
                        : null,
                    child: Text(
                      c.busy
                          ? 'Working…'
                          : active == null
                          ? 'Sign in with Google'
                          : 'Reconnect Google',
                    ),
                  ),
                  if (active != null)
                    OutlinedButton(
                      onPressed: c.busy || !c.loaded
                          ? null
                          : () async {
                              final confirmed = await showDialog<bool>(
                                context: context,
                                builder: (context) => AlertDialog(
                                  title: const Text('Disconnect Google here?'),
                                  content: const Text(
                                    'Cached mail and other devices stay connected. This device stops using the saved Google connection.',
                                  ),
                                  actions: [
                                    TextButton(
                                      onPressed: () =>
                                          Navigator.pop(context, false),
                                      child: const Text('Cancel'),
                                    ),
                                    TextButton(
                                      onPressed: () =>
                                          Navigator.pop(context, true),
                                      child: const Text('Disconnect'),
                                    ),
                                  ],
                                ),
                              );
                              if (confirmed == true) await c.disconnect();
                            },
                      child: const Text('Disconnect…'),
                    ),
                  if (c.cleanupPending)
                    OutlinedButton(
                      onPressed: c.busy || !c.loaded ? null : c.disconnect,
                      child: const Text('Retry cleanup'),
                    ),
                ],
              ),
              if (c.saving)
                const Text(
                  'Saving Google choices…',
                  style: TextStyle(fontSize: 12),
                ),
              if (c.notice case final String notice)
                Padding(
                  padding: const EdgeInsets.only(top: 12),
                  child: Text(notice),
                ),
              if (c.error case final String error) ...[
                const SizedBox(height: 12),
                Semantics(
                  liveRegion: true,
                  child: Text(
                    error,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                ),
                if (!c.loaded)
                  TextButton(
                    onPressed: c.busy ? null : c.load,
                    child: const Text('Retry reading Google connection'),
                  ),
                if (c.loaded && c.choicesUnsaved && !c.busy)
                  TextButton(
                    onPressed: c.saveChoices,
                    child: const Text('Retry saving choices'),
                  ),
              ],
            ],
          ),
        ),
      );
    },
  );
  String _permissions(GooglePermissions p) => [
    p.drive ? 'Drive app data' : 'Drive off',
    switch (p.calendar) {
      GoogleCalendarPermission.off => 'Calendar off',
      GoogleCalendarPermission.read => 'Calendar read only',
      GoogleCalendarPermission.edit => 'Calendar read and edit',
    },
  ].join(' · ');
}
