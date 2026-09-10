import 'package:flutter/material.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/profile_discovery.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/google_fixture.dart';
import 'support/profile_enrollment_fixture.dart';
import 'support/profile_sync_fixture.dart';

/// Preview entry for the saved sync scenario: an applied profile, a remote
/// appearance change waiting on the fixture, and a 52-version conflict once the
/// device edits the same preference. No live provider is contacted.
void main() {
  WidgetsFlutterBinding.ensureInitialized();
  const permission = GooglePermissions(drive: true);
  final google = GoogleConnection(
    FixtureGoogleAuthorization(),
    MemoryGoogleStore(
      const GoogleConnectionState(
        requested: permission,
        active: GoogleConnectionRecord(
          FixtureGoogleAuthorization.subject,
          FixtureGoogleAuthorization.email,
          permission,
          FixtureGoogleAuthorization.application,
        ),
      ),
    ),
  );
  final mail = EnrollmentMail();
  final fixture = FixtureProfileSync(mail, accounts: 0, completed: true)
    ..conflictVersions = 52
    ..lostConfirmOnce = true;
  fixture.pendingRemote['appearance'] = 'Dark';
  // The other device changes the theme concurrently with the saved flow's own
  // next two theme changes, whatever foreground ticks ran in between.
  fixture.concurrent['appearance'] = ['System', 'System'];
  final discovery = ProfileDiscovery(
    google,
    fixture,
    namespace: 'so.shep.fixture',
  );
  final workspace = Workspace(
    mail,
    DeviceSettings(),
    google: google,
    profileDiscovery: discovery,
  );
  runApp(ShepApp(workspace: workspace));
  workspace.initialize();
}
