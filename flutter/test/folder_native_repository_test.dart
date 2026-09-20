import 'dart:io';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/credentials.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/folder_creations.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';

class LockedFolderCredentials implements CredentialStore {
  int reads = 0;
  @override
  Future<String?> read(String account, bool smtp) async {
    reads++;
    throw StateError('locked');
  }

  @override
  Future<void> save(String account, String incoming, String smtp) async =>
      throw StateError('unused');
  @override
  Future<void> remove(String account) async => throw StateError('unused');
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    final name = Platform.isWindows
        ? 'shep_mobile_native.dll'
        : Platform.isMacOS
        ? 'libshep_mobile_native.dylib'
        : 'libshep_mobile_native.so';
    await ShepNative.init(
      externalLibrary: ExternalLibrary.open(
        'build/native_assets/${Platform.operatingSystem}/$name',
      ),
    );
  });
  const account = MailAccount(
    id: 'folders-pop',
    name: 'Personal',
    email: 'alex@example.test',
    host: 'mail.example.test',
    port: 995,
    username: 'alex',
    smtpHost: 'smtp.example.test',
    smtpPort: 465,
    protocol: 'Pop3',
  );

  test(
    'actual FFI persists folder admission and completes POP3 without credential access',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-folder-ffi-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final path = '${directory.path}/mail.sqlite';
      final credentials = LockedFolderCredentials();
      final repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      await repository.call({
        'op': 'save_account',
        'account': account.toJson(),
      });
      await repository.initialize();
      final options = await repository.folderOptions();
      final id = FolderCreations.identity();
      final admitted = await repository.admitFolder(
        id,
        options.single,
        null,
        'Projects',
      );
      expect(admitted.status, 'queued');
      expect(admitted.data['target'], isNull);
      expect(credentials.reads, 0);
      final reopened = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      await reopened.initialize();
      expect((await reopened.folderCreations()).single.id, id);
      final duplicate = await reopened.admitFolder(
        id,
        options.single,
        null,
        'Projects',
      );
      expect(duplicate.revision, admitted.revision);
      final completed = await reopened.executeFolder(duplicate);
      expect(completed.status, 'succeeded');
      expect(reopened.folderNames[account.id], contains('Projects'));
      expect(credentials.reads, 0);
      await expectLater(
        reopened.admitFolder(id, options.single, null, 'Different'),
        throwsA(anything),
      );
      expect((await reopened.folderCreations()).single.status, 'succeeded');
    },
  );

  test('FFI durable cancellation prevents later provider execution', () async {
    final directory = await Directory.systemTemp.createTemp(
      'shep-folder-cancel-',
    );
    addTearDown(() => directory.delete(recursive: true));
    final credentials = LockedFolderCredentials();
    final repository = await NativeRepository.open(
      '${directory.path}/mail.sqlite',
      credentials: credentials,
    );
    await repository.call({'op': 'save_account', 'account': account.toJson()});
    await repository.initialize();
    final option = (await repository.folderOptions()).single;
    final admitted = await repository.admitFolder(
      FolderCreations.identity(),
      option,
      null,
      'Cancelled',
    );
    final cancelled = await repository.decideFolder(admitted, 'cancel');
    expect(cancelled.status, 'cancelled');
    await expectLater(repository.executeFolder(admitted), throwsA(anything));
    expect((await repository.folderCreations()).single.status, 'cancelled');
    expect(credentials.reads, 0);
  });
}
