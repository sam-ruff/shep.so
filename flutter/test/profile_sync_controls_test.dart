import 'package:flutter_test/flutter_test.dart';
import 'support/profile_sync_controls.dart';

void main() {
  testWidgets(
    'sync switches, per-field choices, paged conflict decisions and lost receipts use actual controls',
    profileSyncControls,
  );
  testWidgets(
    'disconnecting Google during a held cycle pauses sync locally',
    profileSyncDisconnectControls,
  );
  testWidgets('review footer clears the device navigation inset', (
    tester,
  ) async {
    tester.view.padding = const FakeViewPadding(bottom: 180);
    addTearDown(tester.view.resetPadding);
    await profileSyncControls(tester);
  });
}
