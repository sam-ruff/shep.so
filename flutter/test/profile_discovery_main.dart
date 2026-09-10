import 'package:flutter/material.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/profile_discovery.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/google_fixture.dart';
import 'support/profile_discovery_fixture.dart';
import 'support/preview_repository.dart';

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
  final discovery = ProfileDiscovery(
    google,
    FixtureProfileDiscovery()..failNext = true,
    namespace: 'so.shep.fixture',
  );
  final workspace = Workspace(
    PreviewRepository(),
    DeviceSettings(),
    google: google,
    profileDiscovery: discovery,
  );
  runApp(ShepApp(workspace: workspace));
  workspace.initialize();
}
