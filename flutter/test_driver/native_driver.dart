import 'dart:io';
import 'package:integration_test/integration_test_driver_extended.dart';

Future<void> main() async {
  final report =
      Platform.environment['SHEP_NATIVE_REPORT'] ?? 'integration-native-result';
  if (!RegExp(r'^[a-z0-9-]+$').hasMatch(report)) {
    throw ArgumentError('Invalid report name');
  }
  final output = Directory('../artifacts/flutter/native');
  await output.create(recursive: true);
  await integrationDriver(
    onScreenshot: (name, bytes, [args]) async {
      if (!RegExp(r'^[a-z0-9-]+$').hasMatch(name)) return false;
      await File('${output.path}/$name.png').writeAsBytes(bytes);
      return true;
    },
    responseDataCallback: (data) => writeResponseData(
      data,
      destinationDirectory: output.path,
      testOutputFilename: report,
    ),
    writeResponseOnFailure: true,
  );
}
