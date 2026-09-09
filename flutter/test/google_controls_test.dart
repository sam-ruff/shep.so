import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'support/google_controls_scenario.dart';

void main() {
  testWidgets(
    'compact Google consent, retained cancellation and durable cleanup controls',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await googleConsentControls(tester);
    },
  );
  testWidgets('browse and change permissions during pending Google consent', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(900, 640);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await googlePendingControls(tester);
  });
}
