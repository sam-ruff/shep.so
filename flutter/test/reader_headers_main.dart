import 'package:flutter/material.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/reader_headers_formatted_repository.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  final workspace = Workspace(HeaderFormattedRepository(), DeviceSettings());
  runApp(ShepApp(workspace: workspace));
  workspace.initialize();
}
