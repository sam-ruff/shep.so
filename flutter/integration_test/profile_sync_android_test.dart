import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import '../test/support/profile_sync_controls.dart';

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  var converted = false;
  final completed = <String>[];
  setUp(() => converted = false);
  Future<void> capture(WidgetTester tester, String name) async {
    if (Platform.isAndroid && !converted) {
      await binding.convertFlutterSurfaceToImage();
      converted = true;
    }
    await tester.pumpAndSettle();
    await binding.takeScreenshot(name);
  }

  testWidgets('native sync controls, conflict decisions and lost receipts', (
    tester,
  ) async {
    await profileSyncControls(tester, capture: (name) => capture(tester, name));
    completed.add('sync-controls-conflicts-receipts');
    binding.reportData = {...?binding.reportData, 'profile_sync': completed};
  });
  testWidgets('native sync pauses locally when Google disconnects', (
    tester,
  ) async {
    await profileSyncDisconnectControls(
      tester,
      capture: (name) => capture(tester, name),
    );
    completed.add('sync-disconnect-pause');
    binding.reportData = {...?binding.reportData, 'profile_sync': completed};
  });
}
