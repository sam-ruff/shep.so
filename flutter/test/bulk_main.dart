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
    // Steps take long enough for Playwright and UiAutomator2 polling to
    // observe progress, Pause and Resume before the group completes.
    bulkPreviewRepository(
      delay: const Duration(milliseconds: 350),
      stepDelay: const Duration(
        milliseconds: int.fromEnvironment(
          'SHEP_BULK_STEP_MS',
          defaultValue: 200,
        ),
      ),
    ),
    DeviceSettings(),
  );
  runApp(ShepApp(workspace: workspace));
  workspace.initialize();
}
