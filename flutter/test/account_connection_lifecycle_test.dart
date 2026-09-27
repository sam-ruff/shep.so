import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/preferences.dart';
import 'package:shep_mobile/ui/theme.dart';

import 'support/paged_repository.dart';

const account = MailAccount(
  id: 'account-one',
  name: 'Personal',
  email: 'sam@example.test',
  host: 'imap.example.test',
  port: 993,
  username: 'sam',
  smtpHost: 'smtp.example.test',
  smtpPort: 465,
);

class MemorySettings implements SettingsStore {
  @override
  Future<Preferences> read() async => const Preferences();
  @override
  Future<void> write(Preferences preferences) async {}
}

class ConnectionRepository extends PagedRepository
    implements DurableAccountRepository {
  @override
  List<MailAccount> mailAccounts = [account];
  @override
  List<AccountConnectionAttempt> connectionAttempts = [];
  final Completer<void> execution = Completer<void>();
  Completer<void>? failStarted, failRelease;
  Completer<void>? admissionRelease;
  int executions = 0;
  Object? failure;

  @override
  Future<AccountConnectionAttempt> admitConnection(
    String attempt,
    MailAccount account,
  ) async {
    final value = AccountConnectionAttempt(
      id: attempt,
      account: account,
      status: 'saving',
    );
    connectionAttempts = [value];
    if (admissionRelease != null) await admissionRelease!.future;
    return value;
  }

  @override
  Future<void> executeConnection(
    AccountConnectionAttempt attempt,
    String incoming,
    String smtp, {
    bool Function()? canDispatch,
  }) async {
    executions++;
    await execution.future;
    if (failure case final failure?) throw failure;
    connectionAttempts = [];
  }

  @override
  Future<void> refreshConnectionAttempts() async {}

  @override
  Future<void> failConnection(String attempt, String error) async {
    failStarted?.complete();
    if (failRelease != null) await failRelease!.future;
  }

  @override
  Future<void> abandonConnection(String attempt) async {
    connectionAttempts = [];
  }
}

void main() {
  for (final dispose in [false, true]) {
    test(
      'held admission cannot dispatch after ${dispose ? 'dispose' : 'background'}',
      () async {
        final repository = ConnectionRepository()
          ..admissionRelease = Completer<void>();
        final workspace = Workspace(repository, MemorySettings());
        final connecting = workspace.connectAccount(
          account,
          'incoming',
          'smtp',
        );
        await Future<void>.delayed(Duration.zero);
        if (dispose) {
          workspace.dispose();
        } else {
          workspace.setForeground(false);
          workspace.setForeground(true);
        }
        repository.admissionRelease!.complete();
        expect(await connecting, false);
        expect(repository.executions, 0);
        expect(repository.connectionAttempts, hasLength(1));
        if (!dispose) workspace.dispose();
      },
    );
  }
  for (final brightness in Brightness.values) {
    testWidgets('compact connection recovery in ${brightness.name}', (
      tester,
    ) async {
      await tester.binding.setSurfaceSize(const Size(390, 700));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await (FontLoader(
        'Roboto',
      )..addFont(rootBundle.load('assets/Roboto-Regular.ttf'))).load();
      await (FontLoader('NotoSans')
            ..addFont(rootBundle.load('assets/NotoSans-Regular.ttf'))
            ..addFont(rootBundle.load('assets/NotoSans-SemiBold.ttf')))
          .load();
      final repository = ConnectionRepository()
        ..connectionAttempts = const [
          AccountConnectionAttempt(
            id: 'recover-connection',
            account: account,
            status: 'failed',
            error: 'Connection check failed. The previous account was kept.',
          ),
        ];
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await tester.pumpWidget(
        MaterialApp(
          theme: shepTheme(brightness),
          home: Scaffold(
            body: ListenableBuilder(
              listenable: workspace,
              builder: (_, _) => PreferencesView(workspace: workspace),
            ),
          ),
        ),
      );
      await tester.scrollUntilVisible(
        find.byKey(const ValueKey('connection-recover-connection')),
        300,
        scrollable: find.byType(Scrollable).first,
      );
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      await expectLater(
        find.byType(Scaffold),
        matchesGoldenFile('goldens/connection_recovery_${brightness.name}.png'),
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(find.text('Re-enter'), findsNothing);
      expect(repository.connectionAttempts, isEmpty);
    });
  }
  testWidgets(
    'durable admission remains visible across navigation and failure',
    (tester) async {
      final repository = ConnectionRepository();
      final workspace = Workspace(repository, MemorySettings());
      await workspace.connectAccount(account, 'incoming', 'smtp');
      await tester.pumpWidget(
        MaterialApp(
          home: ListenableBuilder(
            listenable: workspace,
            builder: (_, _) => PreferencesView(workspace: workspace),
          ),
        ),
      );
      await tester.drag(
        find.byKey(const ValueKey('preferences-list')),
        const Offset(0, -700),
      );
      await tester.pump();
      expect(find.text('Checking connection'), findsOneWidget);
      repository.failure = StateError('Synthetic credential store failure');
      repository.execution.complete();
      await tester.pump();
      await tester.pump();
      expect(
        find.textContaining('Synthetic credential store failure'),
        findsOneWidget,
      );
      expect(find.text('Re-enter'), findsOneWidget);
    },
  );

  testWidgets(
    'restart status offers password re-entry through the real control',
    (tester) async {
      final repository = ConnectionRepository()
        ..connectionAttempts = const [
          AccountConnectionAttempt(
            id: '00000000-0000-4000-8000-000000000041',
            account: account,
            status: 'reentry',
          ),
        ];
      final workspace = Workspace(repository, MemorySettings());
      await tester.pumpWidget(
        MaterialApp(
          home: ListenableBuilder(
            listenable: workspace,
            builder: (_, _) => PreferencesView(workspace: workspace),
          ),
        ),
      );
      await tester.drag(
        find.byKey(const ValueKey('preferences-list')),
        const Offset(0, -700),
      );
      await tester.pumpAndSettle();
      expect(find.text('Passwords required to continue'), findsOneWidget);
      await tester.tap(find.text('Re-enter'));
      await tester.pumpAndSettle();
      expect(find.text('Reconnect account'), findsOneWidget);
      expect(find.text('Incoming password'), findsOneWidget);
    },
  );

  test(
    'same-attempt retry cannot be overwritten by an older held failure',
    () async {
      final repository = ConnectionRepository()
        ..failure = StateError('Older failure')
        ..failStarted = Completer<void>()
        ..failRelease = Completer<void>();
      final workspace = Workspace(repository, MemorySettings());
      await workspace.connectAccount(account, 'first', 'first');
      final attempt = workspace.connectionAttempts.single;
      repository.execution.complete();
      await repository.failStarted!.future;
      repository.failure = null;
      await workspace.connectAccount(
        account,
        'second',
        'second',
        retry: attempt,
      );
      await Future<void>.delayed(Duration.zero);
      repository.failRelease!.complete();
      await Future<void>.delayed(Duration.zero);
      expect(workspace.connectionAttempts, isEmpty);
    },
  );

  test(
    'removed account filters a late repository attempt projection',
    () async {
      final repository = ConnectionRepository();
      final workspace = Workspace(repository, MemorySettings());
      await workspace.accountRemoved(account.id);
      repository.connectionAttempts = const [
        AccountConnectionAttempt(
          id: '00000000-0000-4000-8000-000000000045',
          account: account,
          status: 'failed',
        ),
      ];
      expect(workspace.connectionAttempts, isEmpty);
    },
  );
}
