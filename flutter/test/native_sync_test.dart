import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/credentials.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/src/rust/api.dart';

class UnusedProfile implements MobileProfile {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class HeldCredentials implements CredentialStore {
  Completer<String?>? held;
  Object? failure;
  final reads = <String>[];
  @override
  Future<String?> read(String account, bool smtp) async {
    reads.add(account);
    if (failure case final Object error) throw error;
    return held == null ? 'fixture-secret' : held!.future;
  }

  @override
  Future<void> save(String account, String incoming, String smtp) async {}
  @override
  Future<void> remove(String account) async {}
}

const account = MailAccount(
  id: 'personal',
  name: 'Personal',
  email: 'a@example.test',
  host: 'mail.example.test',
  port: 993,
  username: 'a',
  smtpHost: 'smtp.example.test',
  smtpPort: 465,
);

class ControlledSync extends NativeRepository {
  ControlledSync(HeldCredentials credentials)
    : super(UnusedProfile(), credentials);
  String slot = 'first';
  List<MailAccount> accounts = [account];
  String? failingAccount, heldAccount;
  bool removed = false;
  Object? syncFailure;
  final requests = <Map<String, Object?>>[];
  Completer<dynamic>? target, provider, snapshot;
  Map<String, dynamic> state() => {
    'accounts': removed ? [] : [for (final value in accounts) value.toJson()],
    'folders': <String, dynamic>{},
    'reconnect': [],
    'incoming_slots': removed
        ? {}
        : {for (final value in accounts) value.id: slot},
  };
  @override
  Future<dynamic> call(Map<String, Object?> request) async {
    requests.add(request);
    switch (request['op']) {
      case 'accounts':
        final held = snapshot;
        snapshot = null;
        return held == null ? state() : held.future;
      case 'credential_target':
        return target == null ? {'slot': slot} : target!.future;
      case 'sync':
        if (request['account'] == failingAccount) {
          throw const MailOperationFailure('Public account failure');
        }
        if (heldAccount != null && request['account'] != heldAccount) {
          return {'skipped_large': 0};
        }
        if (syncFailure case final Object failure) throw failure;
        return provider == null ? {'skipped_large': 0} : provider!.future;
      default:
        throw StateError('Unexpected operation ${request['op']}');
    }
  }
}

Future<void> until(bool Function() ready) async {
  for (var attempts = 0; attempts < 100 && !ready(); attempts++) {
    await Future<void>.delayed(Duration.zero);
  }
  expect(ready(), isTrue);
}

void main() {
  test(
    'native account outcomes arrive before a later account finishes',
    () async {
      final repo = ControlledSync(HeldCredentials());
      repo.accounts = [
        account,
        MailAccount.fromJson({
          ...account.toJson(),
          'id': 'work',
          'name': 'Work',
        }),
      ];
      repo.failingAccount = 'personal';
      repo.heldAccount = 'work';
      repo.provider = Completer<dynamic>();
      await repo.refreshProfileAccounts();
      final observed = <String>[];
      final refresh = repo.refreshIncoming(
        onResult: (result) => observed.add(result.account.id),
      );
      await until(
        () => repo.requests.any(
          (request) => request['op'] == 'sync' && request['account'] == 'work',
        ),
      );
      expect(observed, ['personal']);
      repo.provider!.complete({'skipped_large': 0});
      expect((await refresh).length, 2);
      expect(observed, ['personal', 'work']);
    },
  );
  test(
    'outcomes retain the exact dispatched credential slot and safe public error',
    () async {
      final repo = ControlledSync(HeldCredentials());
      await repo.refreshProfileAccounts();
      final success = await repo.refreshIncoming();
      expect(success.single.account.identity.slot, 'first');
      expect(success.single.error, isNull);
      expect(
        repo.requests
            .where((request) => request['op'] == 'sync')
            .single['credential_slot'],
        'first',
      );
      repo.syncFailure = const MailOperationFailure(
        'Public connection failure',
      );
      expect(
        (await repo.refreshIncoming()).single.error,
        'Public connection failure',
      );
      repo.syncFailure = StateError('private fixture-secret provider trace');
      expect(
        (await repo.refreshIncoming()).single.error,
        'Could not refresh mail. Check the connection and retry.',
      );
    },
  );

  test(
    'reconnect during credential reads cannot dispatch the old password',
    () async {
      final credentials = HeldCredentials();
      final actual = ControlledSync(credentials);
      await actual.refreshProfileAccounts();
      credentials.held = Completer<String?>();
      final refresh = actual.refreshIncoming();
      await until(() => credentials.reads.isNotEmpty);
      actual.slot = 'replacement';
      await actual.refreshProfileAccounts();
      credentials.held!.complete('old-secret');
      expect(await refresh, isEmpty);
      expect(
        actual.requests.where((request) => request['op'] == 'sync'),
        isEmpty,
      );
    },
  );

  test(
    'foreground expiry after target wait prevents credential access and provider dispatch',
    () async {
      final credentials = HeldCredentials();
      final actual = ControlledSync(credentials);
      await actual.refreshProfileAccounts();
      actual.target = Completer<dynamic>();
      var foreground = true;
      final refresh = actual.refreshIncoming(canDispatch: () => foreground);
      await until(
        () => actual.requests.any(
          (request) => request['op'] == 'credential_target',
        ),
      );
      foreground = false;
      actual.target!.complete({'slot': 'first'});
      expect(await refresh, isEmpty);
      expect(credentials.reads, isEmpty);
      expect(
        actual.requests.where((request) => request['op'] == 'sync'),
        isEmpty,
      );
    },
  );

  test(
    'removal drops a provider reply and an older account snapshot cannot replace newer identity',
    () async {
      final repo = ControlledSync(HeldCredentials());
      await repo.refreshProfileAccounts();
      repo.provider = Completer<dynamic>();
      final refresh = repo.refreshIncoming();
      await until(
        () => repo.requests.any((request) => request['op'] == 'sync'),
      );
      repo.removed = true;
      await repo.refreshProfileAccounts();
      repo.provider!.complete({'skipped_large': 0});
      expect(await refresh, isEmpty);
      repo.removed = false;
      final oldState = repo.state(), held = Completer<dynamic>();
      repo.snapshot = held;
      final older = repo.refreshProfileAccounts();
      repo.slot = 'new';
      await repo.refreshProfileAccounts();
      held.complete(oldState);
      await older;
      expect(repo.incomingAccounts.single.identity.slot, 'new');
    },
  );

  test(
    'credential-store exceptions stay private and missing password has visible recovery',
    () async {
      final credentials = HeldCredentials();
      final repo = ControlledSync(credentials);
      await repo.refreshProfileAccounts();
      credentials.failure = StateError('private store path fixture-secret');
      expect(
        (await repo.refreshIncoming()).single.error,
        'The device credential store is unavailable. Unlock the device and retry; cached mail is available.',
      );
      credentials.failure = null;
      credentials.held = Completer<String?>()..complete(null);
      expect(
        (await repo.refreshIncoming()).single.error,
        contains('Reconnect this account in Preferences'),
      );
      expect(
        repo.requests.where((request) => request['op'] == 'sync'),
        isEmpty,
      );
    },
  );
}
