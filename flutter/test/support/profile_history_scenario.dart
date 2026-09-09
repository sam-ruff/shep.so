import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/data/profile_history.dart';

// Production FFI/SQLite scenario shared by host and isolated Android runs.
Future<void> exerciseProfileHistory(
  NativeRepository first,
  NativeRepository second,
) async {
  final original = await second.call({'op': 'accounts'});
  const binding = ProfileHistoryBinding(
    namespace: 'so.shep.fixture',
    principal: 'drive:fixture-owner',
    profile: '00000000-0000-0000-0000-000000000001',
    generation: '00000000-0000-0000-0000-000000000002',
  );
  final desktop = NativeProfileHistory(first, binding);
  final phone = NativeProfileHistory(second, binding);
  addTearDown(() async {
    await desktop.close();
    await phone.close();
  });
  Future<Map<String, Object?>> prepare(
    NativeProfileHistory history,
    String operation,
    String appearance,
  ) async => {
    'operation': operation,
    'expected_revision': (await history.state()).revision,
    'changes': <Map<String, Object?>>[
      {'kind': 'setting', 'key': 'appearance', 'value': appearance},
    ],
  };
  // This fixture copies immutable bytes; it is not a real Google upload or
  // proof that the supplied binding has passed provider authentication.
  Future<void> copy(
    NativeProfileHistory source,
    NativeProfileHistory destination,
  ) async {
    final upload = (await source.nextUpload())!;
    await destination.importRecord(upload['record'] as String);
    final operation = upload['operation'] as String;
    await source.reserve(operation, 'fixture-$operation');
    await source.confirm(
      operation,
      'fixture-$operation',
      upload['sha256'] as String,
    );
  }

  final initial = await prepare(
    desktop,
    '00000000-0000-0000-0000-000000000010',
    'Light',
  );
  const accountId = '00000000-0000-0000-0000-000000000020';
  (initial['changes'] as List).add({
    'kind': 'account_connection',
    'account': {
      'id': accountId,
      'email': 'shared@example.test',
      'protocol': 'Imap',
      'host': 'imap.example.test',
      'port': 993,
      'username': 'shared',
      'incoming_security': 'Tls',
      'incoming_auth': 'Password',
      'smtp_host': 'smtp.example.test',
      'smtp_port': 465,
      'smtp_username': 'outgoing',
      'smtp_security': 'Tls',
      'smtp_auth': 'Automatic',
      'smtp_separate_password': true,
      'sent_copy': 'ServerManaged',
      'sent_folder': 'Sent',
      'future_connection_option': {'retain': true},
    },
  });
  await desktop.edit(initial);
  await copy(desktop, phone);
  await phone.edit(
    await prepare(phone, '00000000-0000-0000-0000-000000000011', 'Dark'),
  );
  await desktop.edit(
    await prepare(desktop, '00000000-0000-0000-0000-000000000012', 'System'),
  );
  await copy(desktop, phone);
  await copy(phone, desktop);
  expect((await desktop.state()).conflicts, 1);
  expect((await phone.state()).conflicts, 1);
  expect(
    (await desktop.fields()).singleWhere(
      (f) => f['target'] == 'setting:appearance',
    )['conflict'],
    true,
  );
  final accountVersion =
      (await phone.versions(
            'account:$accountId:connection',
          )).single['operation']
          as String;
  final account =
      (await phone.value(
            'account:$accountId:connection',
            accountVersion,
          ))['account']
          as Map;
  expect(account['smtp_separate_password'], true);
  expect(account['future_connection_option'], {'retain': true});
  final versions = await desktop.versions('setting:appearance');
  expect(versions, hasLength(2));
  final resolution = await prepare(
    desktop,
    '00000000-0000-0000-0000-000000000013',
    'Dark',
  );
  await expectLater(
    desktop.edit(resolution),
    throwsA(isA<MailOperationFailure>()),
  );
  resolution['resolutions'] = [
    {
      'target': 'setting:appearance',
      'versions': versions.map((v) => v['operation']).toList(),
    },
  ];
  await desktop.edit(resolution);
  final queued = await desktop.nextUpload();
  final device = (await desktop.state()).device;
  await desktop.close();
  expect((await desktop.state()).device, device);
  expect(await desktop.nextUpload(), queued);
  await desktop.edit(
    resolution,
  ); // Lost reply: the same edit is still one operation.
  expect((await desktop.state()).operations, 4);
  await copy(desktop, phone);
  expect((await phone.state()).conflicts, 0);
  final current =
      (await phone.versions('setting:appearance')).single['operation']
          as String;
  expect((await phone.value('setting:appearance', current))['value'], 'Dark');
  expect(await second.call({'op': 'accounts'}), original);
}
