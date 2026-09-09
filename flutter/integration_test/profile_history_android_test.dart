import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shep_mobile/data/credentials.dart';
import 'package:shep_mobile/data/native_repository.dart';
import '../test/support/profile_history_scenario.dart';

class _LockedCredentials implements CredentialStore {
  int requests = 0;
  Never refuse() {
    requests++;
    throw StateError('Fictional device credentials are locked.');
  }

  @override
  Future<String?> read(String account, bool smtp) async => refuse();
  @override
  Future<void> save(String account, String incoming, String smtp) async =>
      refuse();
  @override
  Future<void> remove(String account) async => refuse();
}

void main() {
  final report = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets(
    'two native profile histories merge and reopen without device credentials',
    (tester) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(body: Text('Isolated profile history verification')),
        ),
      );
      final root = await (await getTemporaryDirectory()).createTemp(
        'shep-history-',
      );
      addTearDown(() => root.delete(recursive: true));
      final credentials = _LockedCredentials();
      final first = await NativeRepository.open(
        '${root.path}/first.sqlite',
        credentials: credentials,
      );
      final second = await NativeRepository.open(
        '${root.path}/second.sqlite',
        credentials: credentials,
      );
      await exerciseProfileHistory(first, second);
      expect(credentials.requests, 0);
      report.reportData = {
        ...?report.reportData,
        'profile_history': ['two-device-conflicts-restart'],
      };
    },
  );
}
