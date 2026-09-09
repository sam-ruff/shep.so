import 'package:flutter/material.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/preview_repository.dart';
import 'support/google_fixture.dart';
import 'package:shep_mobile/model/google_connection.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  final workspace = Workspace(
    PreviewRepository(fail: const bool.fromEnvironment('SHEP_FAIL_ACTIONS')),
    DeviceSettings(),
    google: GoogleConnection(
      FixtureGoogleAuthorization()
        ..failEditingOnce = true
        ..delay = const Duration(milliseconds: 350),
      PreviewGoogleStore(),
    ),
  );
  runApp(ShepApp(workspace: workspace));
  workspace.initialize();
}
