import 'package:flutter/material.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/bulk_fixture.dart';

/// Browser/Appium entry for the saved group action scenarios: the preview
/// workspace plus 125 fictional Inbox messages and a synthetic journal.
void main() {
  WidgetsFlutterBinding.ensureInitialized();
  final workspace = Workspace(
    bulkPreviewRepository(
      delay: const Duration(milliseconds: 350),
      stepDelay: const Duration(milliseconds: 40),
    ),
    DeviceSettings(),
  );
  runApp(ShepApp(workspace: workspace));
  workspace.initialize();
}
