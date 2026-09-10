import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'support/reader_actions_scenario.dart';

void main() {
  for (final dark in [false, true]) {
    testWidgets(
      'reader footer survives loading and held touch (${dark ? 'dark' : 'light'})',
      (tester) async {
        tester.view.physicalSize = const Size(412, 892);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        await readerActionsScenario(tester, dark: dark);
      },
    );
  }
  testWidgets(
    'reader footer remains reachable with compact scaled text and bottom safe area',
    (tester) async {
      tester.view.physicalSize = const Size(360, 800);
      tester.view.devicePixelRatio = 1;
      tester.view.padding = const FakeViewPadding(bottom: 34);
      tester.platformDispatcher.textScaleFactorTestValue = 1.5;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPadding);
      addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      await readerActionsScenario(tester, dark: true);
    },
  );
}
