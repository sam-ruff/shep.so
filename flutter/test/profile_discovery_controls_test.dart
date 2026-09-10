import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'support/profile_discovery_controls.dart';

void main() {
  testWidgets(
    'compact profile discovery failure, retry, pagination and appearance controls',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await profileDiscoveryControls(tester);
    },
  );
  testWidgets(
    'pending discovery can pause while actual mail controls remain usable',
    (tester) async {
      tester.view.physicalSize = const Size(900, 640);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await profileDiscoveryPendingControls(tester);
    },
  );
}
