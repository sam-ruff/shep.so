import '../test/support/sent_handover_scenario.dart';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/credentials.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/main.dart' as production;
import '../test/native_repository_test.dart' show FixtureCredentials;
import '../test/workspace_test.dart' show MemorySettings;
import '../test/support/paged_repository.dart';

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  bool surfaceConverted = false;
  setUp(() => surfaceConverted = false);
  Future<void> capture(WidgetTester tester, String name) async {
    if (Platform.isAndroid && !surfaceConverted) {
      await binding.convertFlutterSurfaceToImage();
      surfaceConverted = true;
    }
    await tester.pumpAndSettle();
    await binding.takeScreenshot(name);
  }

  Future<void> wait(WidgetTester tester, bool Function() ready) async {
    final end = DateTime.now().add(const Duration(seconds: 20));
    while (!ready()) {
      if (DateTime.now().isAfter(end)) {
        fail('Native UI did not reach the expected state');
      }
      await tester.pump(const Duration(milliseconds: 100));
    }
    await tester.pumpAndSettle();
  }

  testWidgets('Sent handover preserves reader identity, late body and Undo', (
    tester,
  ) async {
    await sentHandoverScenario(
      tester,
      capture: (name) => capture(tester, name),
    );
  });

  testWidgets(
    'paged swipe Undo stays immediate during transport and survives folder refresh',
    (tester) async {
      final repository = PagedRepository();
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      await tester.drag(
        find.byKey(const ValueKey('mail-1')),
        const Offset(-350, 0),
      );
      await tester.pumpAndSettle();
      expect(repository.jobs.length, 1);
      expect(find.text('A little room for good ideas'), findsNothing);
      await tester.tap(find.text('Undo'));
      await tester.pumpAndSettle();
      expect(find.text('A little room for good ideas'), findsOneWidget);
      expect(repository.jobs.length, 1); // Undo is queued behind the held MOVE.
      repository.jobs[0].complete();
      await wait(tester, () => repository.jobs.length == 2);
      repository.jobs[1].complete();
      await wait(tester, () => workspace.pending == 0);
      await tester.drag(
        find.byKey(const ValueKey('mail-1')),
        const Offset(-350, 0),
      );
      await wait(tester, () => repository.jobs.length == 3);
      repository.jobs[2].complete();
      await wait(tester, () => workspace.pending == 0);
      await tester.tap(find.byTooltip('Refresh'));
      await tester.pumpAndSettle();
      await wait(tester, () => !workspace.syncing);
      expect(find.text('A little room for good ideas'), findsNothing);
      await tester.tap(find.text('Undo'));
      await tester.pumpAndSettle();
      expect(find.text('A little room for good ideas'), findsOneWidget);
      await wait(tester, () => repository.jobs.length == 4);
      repository.jobs[3].complete();
      await wait(tester, () => workspace.pending == 0);
      await capture(tester, 'paged-swipe-undo');
      expect(workspace.error, isNull);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'native SQLite draft save, restart, edit and permanent discard through controls',
    (tester) async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-native-ui-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final path = '${directory.path}/mail.sqlite3';
      final credentials = FixtureCredentials();
      var repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      const account = MailAccount(
        id: 'native-ui',
        name: 'Native test account',
        email: 'alex@example.test',
        host: '127.0.0.1',
        port: 1,
        username: 'fixture',
        smtpHost: '127.0.0.1',
        smtpPort: 1,
        protocol: 'Pop3',
      );
      // Isolated fixture setup happens before the UI starts. All tested actions
      // below use actual Flutter controls and production SQLite/FFI code.
      await repository.call({
        'op': 'save_account',
        'account': account.toJson(),
      });
      var workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Compose'));
      await tester.pumpAndSettle();
      await wait(
        tester,
        () => !tester
            .widget<TextField>(find.widgetWithText(TextField, 'To'))
            .readOnly,
      );
      expect(find.text('alex@example.test'), findsOneWidget);
      await tester.enterText(
        find.widgetWithText(TextField, 'To'),
        'robin@example.test',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Subject'),
        'Native durable draft',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Message'),
        'This text survives reopening the Rust profile.',
      );
      await tester.tap(find.text('Save draft'));
      await wait(tester, () => find.text('Compose').evaluate().isNotEmpty);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
      repository = await NativeRepository.open(path, credentials: credentials);
      workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Open navigation menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Drafts'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Native durable draft'));
      await tester.pumpAndSettle();
      await wait(
        tester,
        () => !tester
            .widget<TextField>(find.widgetWithText(TextField, 'To'))
            .readOnly,
      );
      expect(
        find.text('This text survives reopening the Rust profile.'),
        findsOneWidget,
      );
      await capture(tester, 'native-draft-reopened');
      await tester.enterText(
        find.widgetWithText(TextField, 'Subject'),
        'Revised native draft',
      );
      await tester.tap(find.byTooltip('Discard draft'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Keep draft'));
      await tester.pumpAndSettle();
      expect(find.text('Revised native draft'), findsOneWidget);
      await tester.tap(find.byTooltip('Discard draft'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Discard draft').last);
      await wait(
        tester,
        () => find.text('No saved drafts').evaluate().isNotEmpty,
      );
      await repository.initialize();
      expect(repository.savedDrafts, isEmpty);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
  );

  testWidgets(
    'native connection failure leaves account form usable and credentials untouched',
    (tester) async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-native-connection-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final credentials = FixtureCredentials();
      final repository = await NativeRepository.open(
        '${directory.path}/mail.sqlite3',
        credentials: credentials,
      );
      final workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Preferences'));
      await tester.pumpAndSettle();
      await tester.scrollUntilVisible(
        find.text('Add mail account'),
        300,
        scrollable: find.byType(Scrollable).last,
      );
      await tester.tap(find.text('Add mail account'));
      await tester.pumpAndSettle();
      Future<void> fill(String label, String value) async {
        final finder = find.widgetWithText(TextFormField, label);
        await tester.ensureVisible(finder);
        await tester.pumpAndSettle();
        await tester.enterText(finder, value);
      }

      await fill('Account name', 'Isolated connection test');
      await fill('Email address', 'alex@example.test');
      await fill('Incoming hostname', '127.0.0.1');
      await fill('Incoming port', '1');
      await fill('Incoming username', 'fixture');
      await fill('Incoming password', 'fixture-password');
      await fill('SMTP hostname', '127.0.0.1');
      await fill('SMTP port', '1');
      await tester.scrollUntilVisible(
        find.text('Connect account'),
        250,
        scrollable: find.byType(Scrollable).last,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Connect account'));
      await tester.pump();
      await wait(
        tester,
        () => find.text('Connect account').evaluate().isNotEmpty,
      );
      expect(credentials.values, isEmpty);
      expect(repository.mailAccounts, isEmpty);
      expect(
        find.textContaining('Could not verify the connection'),
        findsOneWidget,
      );
      await capture(tester, 'native-account-retry');
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
  );

  testWidgets('device credential pairs roundtrip in isolated preview storage', (
    tester,
  ) async {
    const store = DeviceCredentials();
    final id = 'shep-e2e-${DateTime.now().microsecondsSinceEpoch}';
    addTearDown(() => store.remove(id));
    await store.save(id, 'fixture-incoming', 'fixture-smtp');
    expect(await store.read(id, false), 'fixture-incoming');
    expect(await store.read(id, true), 'fixture-smtp');
    await store.save(id, 'fixture-new-incoming', 'fixture-new-smtp');
    expect(await store.read(id, true), 'fixture-new-smtp');
    await store.remove(id);
    expect(await store.read(id, false), isNull);
  });
  testWidgets(
    'production entry opens the native cache and account setup without fixture mail',
    (tester) async {
      production.main();
      await wait(
        tester,
        () => find.text('Welcome to Shep').evaluate().isNotEmpty,
      );
      expect(find.text('PREVIEW · FICTIONAL DATA'), findsNothing);
      expect(find.text('A little room for good ideas'), findsNothing);
      await capture(tester, 'native-production-startup');
      await tester.tap(find.text('Preferences'));
      await tester.pumpAndSettle();
      await tester.scrollUntilVisible(
        find.text('Add mail account'),
        300,
        scrollable: find.byType(Scrollable).last,
      );
      await tester.tap(find.text('Add mail account'));
      await tester.pumpAndSettle();
      expect(
        find.widgetWithText(TextFormField, 'Account name'),
        findsOneWidget,
      );
      expect(find.text('Connect account'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
