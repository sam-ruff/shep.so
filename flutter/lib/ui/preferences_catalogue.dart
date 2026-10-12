import '../data/accounts.dart';
import '../model/preferences_search.dart';
import '../model/profile_discovery.dart';
import '../model/workspace.dart';
import 'profile_labels.dart';

List<PreferenceSearchEntry> preferencesCatalogue(Workspace workspace) {
  final entries = [...mobilePreferenceEntries];
  void add(
    String target,
    String label,
    String section,
    String description, [
    String synonyms = '',
  ]) {
    entries.add(
      PreferenceSearchEntry(
        target: target,
        label: label,
        section: section,
        description: description,
        synonyms: synonyms,
      ),
    );
  }

  if (workspace.preferenceSaveError != null) {
    add(
      'preference-save-status',
      'Retry save',
      'Preferences',
      'Save current local preferences',
      'failure error storage',
    );
  }
  if (workspace.accountRepository case final accounts?) {
    add(
      'preference-add-account',
      'Add mail account',
      'Connections',
      'Set up an incoming and outgoing mail connection',
      'IMAP POP3 SMTP server password',
    );
    for (final account in accounts.mailAccounts) {
      add(
        'preference-account-${account.id}',
        account.name,
        'Connections',
        '${account.email} Reconnect mail account',
        'IMAP POP3 SMTP server password incoming outgoing host port username security TLS authentication separate password',
      );
      if (workspace.repository is SentPreferencesRepository) {
        add(
          'preference-sent-${account.id}',
          'Sent copies',
          'Connections',
          '${account.name} ${account.email} Save outgoing mail',
          'filing sent folder automatic local server copy policy',
        );
      }
      if (workspace.repository is AccountRemovalRepository) {
        add(
          'preference-remove-${account.id}',
          'Remove ${account.email}',
          'Connections',
          'Remove this account from this device',
          'delete disconnect',
        );
      }
    }
    for (final attempt in workspace.connectionAttempts) {
      add(
        'connection-${attempt.id}',
        'Cancel connection',
        'Connections',
        '${attempt.account.email} Checking connection',
        'stop pending attempt',
      );
      if (attempt.needsPasswords) {
        add(
          'connection-${attempt.id}',
          'Re-enter passwords',
          'Connections',
          attempt.account.email,
          'retry reconnect',
        );
      }
    }
    if (workspace.repository case final AccountRemovalRepository removal) {
      if (removal.pendingCredentialCleanup > 0) {
        add(
          'preference-credential-cleanup',
          'Saved passwords need cleanup',
          'Connections',
          'Unlock device credential storage, then retry.',
          'retry cleanup keychain',
        );
      }
    }
  }
  if (workspace.google case final google?) {
    add(
      'google-drive',
      'Private Drive storage',
      'Google connection',
      'Only Shep’s app data. Permissions for the next sign-in',
      'backup profiles cloud',
    );
    add(
      'google-calendar-${google.requested.calendar.name}',
      'Calendar access',
      'Google connection',
      'No Calendar access Read calendars Read and edit calendars',
      'permission consent readonly',
    );
    add(
      'google-connect',
      google.active == null ? 'Sign in with Google' : 'Reconnect Google',
      'Google connection',
      'Changing choices does not change saved access until sign-in succeeds.',
      'login authenticate permissions',
    );
    if (google.active != null) {
      add(
        'google-disconnect',
        'Disconnect…',
        'Google connection',
        'Stop using the saved Google connection on this device',
        'sign out logout',
      );
    }
    if (google.cleanupPending) {
      add(
        'google-cleanup',
        'Retry cleanup',
        'Google connection',
        'Clean up the Google connection on this device',
        'sign out logout',
      );
    }
    if (google.error != null && !google.loaded) {
      add(
        'google-retry-read',
        'Retry reading Google connection',
        'Google connection',
        'Read saved access again',
        'error failure',
      );
    }
    if (google.error != null &&
        google.loaded &&
        google.choicesUnsaved &&
        !google.busy) {
      add(
        'google-retry-save',
        'Retry saving choices',
        'Google connection',
        'Save requested Google permissions',
        'error failure',
      );
    }
  }
  if (workspace.supportsCalDav) {
    add(
      'preference-caldav',
      'CalDAV calendars',
      'Connections',
      'Add calendar Calendar URL Username Password Connect Reconnect Remove Enter password Cancel Retry cleanup',
      'calendar server credentials',
    );
    for (final field in ['Calendar URL', 'Username', 'Password']) {
      final target = field == 'Calendar URL' ? 'url' : field.toLowerCase();
      add(
        'caldav-$target',
        field,
        'CalDAV calendars',
        'Connect to a calendar server',
        'credentials calendar',
      );
    }
    for (final connection in workspace.calDavConnections) {
      add(
        'caldav-connection-${connection.id}',
        connection.username,
        'CalDAV calendars',
        '${connection.url} Reconnect Remove',
        'credentials calendar',
      );
    }
    if (workspace.calDavCleanupError != null) {
      add(
        'caldav-cleanup',
        'Credential cleanup waiting',
        'CalDAV calendars',
        'Retry cleanup',
        'unlock device storage password credentials',
      );
    }
    for (final attempt in workspace.calDavAttempts.where(
      (attempt) =>
          const {'prepared', 'probing', 'waiting'}.contains(attempt.status),
    )) {
      add(
        'caldav-attempt-${attempt.id}',
        attempt.status == 'probing' ? 'Checking calendar' : 'Calendar waiting',
        'CalDAV calendars',
        '${attempt.url} ${attempt.username} Enter password Cancel',
        'retry connection',
      );
    }
  }
  if (workspace.profileDiscovery case final discovery?) {
    add(
      'preference-profiles',
      'Saved Google profiles',
      'Profiles and sync',
      'Discover account and settings profiles',
      'shared cloud backup restore',
    );
    if (discovery.supportsSync &&
        workspace.profileApplication != null &&
        discovery.connected) {
      final sync = discovery.sync;
      if (sync != null) {
        add(
          'profile-sync-master',
          'Sync preferences',
          'Profiles and sync',
          'With ${sync.name ?? 'the applied profile'}',
          'automatic synchronise pause',
        );
        for (final field in profileSettingLabels.keys) {
          add(
            'profile-sync-field-$field',
            profileSettingLabel(field),
            'Profiles and sync',
            'Choose whether to sync this preference',
            'shared',
          );
        }
        add(
          'profile-sync-now',
          'Sync now',
          'Profiles and sync',
          'Update this device from the applied profile',
          'refresh synchronise',
        );
        if (sync.reviews > 0) {
          add(
            'profile-sync-reviews',
            'Review preference conflicts',
            'Profiles and sync',
            'Review ${sync.reviews} preference conflicts',
            'resolve versions keep mine',
          );
        }
      } else if (discovery.syncChecked && discovery.canSubscribe) {
        add(
          'profile-sync-subscribe',
          'Keep in sync',
          'Profiles and sync',
          'Preference sync is not set up. Keep this device updated with the applied profile.',
          'subscribe automatic',
        );
      }
    }
  }
  return entries;
}
