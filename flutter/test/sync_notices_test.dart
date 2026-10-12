import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/incoming_sync.dart';
import 'package:shep_mobile/model/sync_notices.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/paged_repository.dart';
import 'workspace_test.dart' show MemorySettings;
import 'mail_activity_visual_test.dart' show loadPreviewFonts;

IncomingAccount incoming(
  String id, {
  String? name,
  String slot = 'original',
  String host = 'mail.example.test',
}) => IncomingAccount(id, name ?? id, (
  protocol: 'Imap',
  host: host,
  port: 993,
  username: id,
  security: 'Tls',
  authentication: 'Password',
  slot: slot,
));

class SyncRepository extends PagedRepository implements IncomingSyncRepository {
  @override
  List<IncomingAccount> incomingAccounts = [incoming('Personal')];
  @override
  List<MailAccount> get mailAccounts => [
    for (final target in incomingAccounts)
      MailAccount(
        id: target.id,
        name: target.name,
        email: '${target.id}@example.test',
        host: target.identity.host,
        port: 993,
        username: target.id,
        smtpHost: 'smtp.example.test',
        smtpPort: 465,
      ),
  ];
  String? failure = 'The server is unavailable. Retry.';
  Completer<List<IncomingResult>>? held;
  int checks = 0;
  @override
  Future<List<IncomingResult>> refreshIncoming({
    bool Function()? canDispatch,
    void Function(IncomingResult)? onResult,
  }) async {
    checks++;
    final results = held != null
        ? await held!.future
        : [
            for (final account in incomingAccounts)
              IncomingResult(account, error: failure),
          ];
    for (final result in results) {
      if (canDispatch?.call() != false) onResult?.call(result);
    }
    return results;
  }
}

class StreamingSyncRepository extends SyncRepository {
  StreamingSyncRepository() {
    incomingAccounts = [incoming('Personal'), incoming('Work')];
  }
  final second = Completer<void>();
  bool snapshotFails = false;
  @override
  Future<List<IncomingResult>> refreshIncoming({
    bool Function()? canDispatch,
    void Function(IncomingResult)? onResult,
  }) async {
    checks++;
    final first = IncomingResult(incomingAccounts.first, error: 'Offline');
    onResult?.call(first);
    await second.future;
    final last = IncomingResult(incomingAccounts.last);
    onResult?.call(last);
    if (snapshotFails) {
      throw const MailOperationFailure('The account list could not reload.');
    }
    return [first, last];
  }
}

void main() {
  test(
    'a completed account failure ages while another check is held; failed snapshot keeps it',
    () async {
      var now = DateTime.utc(2026);
      final repo = StreamingSyncRepository()..snapshotFails = true;
      final workspace = Workspace(repo, MemorySettings(), syncClock: () => now);
      addTearDown(workspace.dispose);
      final refresh = workspace.refresh(automatic: true);
      await Future<void>.delayed(Duration.zero);
      expect(workspace.syncing, isTrue);
      expect(workspace.error, isNull);
      now = now.add(const Duration(seconds: 30));
      expect(workspace.error, 'Personal sync failed: Offline');
      repo.second.complete();
      await refresh;
      expect(workspace.error, 'Personal sync failed: Offline');
    },
  );

  test(
    'next-notice deadline is account-bound and stops after recovery or dismissal',
    () {
      var now = DateTime.utc(2026);
      final notices = SyncNotices(now: () => now), a = incoming('Personal');
      expect(notices.nextNoticeDelay([a]), isNull);
      notices.observe(
        [IncomingResult(a, error: 'Offline')],
        [a],
        automatic: true,
      );
      expect(notices.nextNoticeDelay([a]), const Duration(seconds: 30));
      now = now.add(const Duration(seconds: 29));
      expect(notices.nextNoticeDelay([a]), const Duration(seconds: 1));
      now = now.add(const Duration(seconds: 1));
      expect(notices.nextNoticeDelay([a]), isNull);
      notices.dismiss();
      expect(notices.notice([a]), isNull);
    },
  );
  test(
    'continuous episodes begin at the first observed failure and keep latest error',
    () {
      var now = DateTime.utc(2026);
      final model = SyncNotices(now: () => now), a = incoming('Personal');
      model.observe([IncomingResult(a, error: 'First')], [a], automatic: true);
      now = now.add(const Duration(seconds: 29));
      model.observe([IncomingResult(a, error: 'Latest')], [a], automatic: true);
      expect(model.notice([a]), isNull);
      now = now.add(const Duration(seconds: 1));
      expect(model.notice([a]), 'Personal sync failed: Latest');
      model.observe([IncomingResult(a)], [a], automatic: true);
      expect(model.notice([a]), isNull);
      model.observe([IncomingResult(a, error: 'New')], [a], automatic: true);
      expect(model.notice([a]), isNull);
    },
  );

  test('explicit failures neither start nor reset automatic episodes', () {
    var now = DateTime.utc(2026);
    final model = SyncNotices(now: () => now), a = incoming('Personal');
    model.observe([IncomingResult(a, error: 'Manual')], [a], automatic: false);
    now = now.add(const Duration(minutes: 1));
    expect(model.notice([a]), isNull);
    model.observe(
      [IncomingResult(a, error: 'Automatic')],
      [a],
      automatic: true,
    );
    now = now.add(const Duration(seconds: 20));
    model.observe([IncomingResult(a, error: 'Manual')], [a], automatic: false);
    now = now.add(const Duration(seconds: 10));
    expect(model.notice([a]), 'Personal sync failed: Manual');
    model.observe([IncomingResult(a)], [a], automatic: false);
    expect(model.notice([a]), isNull);
  });

  test('accounts age independently and multiple failures remain explicit', () {
    var now = DateTime.utc(2026);
    final model = SyncNotices(now: () => now),
        a = incoming('Personal'),
        b = incoming('Work');
    model.observe(
      [IncomingResult(a, error: 'Offline')],
      [a, b],
      automatic: true,
    );
    now = now.add(const Duration(seconds: 15));
    model.observe(
      [IncomingResult(b, error: 'Password missing')],
      [a, b],
      automatic: true,
    );
    now = now.add(const Duration(seconds: 15));
    expect(model.notice([a, b]), 'Personal sync failed: Offline');
    now = now.add(const Duration(seconds: 15));
    expect(
      model.notice([a, b]),
      '2 accounts could not refresh:\nPersonal: Offline\nWork: Password missing',
    );
    model.observe([IncomingResult(a)], [a, b], automatic: true);
    expect(model.notice([a, b]), 'Work sync failed: Password missing');
  });

  test(
    'rename retains episodes; reconnect, configuration and removal reject late results',
    () {
      var now = DateTime.utc(2026);
      final model = SyncNotices(now: () => now), old = incoming('a');
      model.observe(
        [IncomingResult(old, error: 'Offline')],
        [old],
        automatic: true,
      );
      now = now.add(const Duration(seconds: 30));
      expect(
        model.notice([incoming('a', name: 'Renamed')]),
        'Renamed sync failed: Offline',
      );
      for (final changed in [
        incoming('a', slot: 'new'),
        incoming('a', host: 'new.example.test'),
      ]) {
        expect(
          model.observe(
            [IncomingResult(old, error: 'Late')],
            [changed],
            automatic: true,
          ),
          isEmpty,
        );
        expect(model.notice([changed]), isNull);
      }
      expect(
        model.observe(
          [IncomingResult(old, error: 'Late')],
          [],
          automatic: true,
        ),
        isEmpty,
      );
    },
  );

  test('dismissal suppresses the continuous episode until recovery', () {
    var now = DateTime.utc(2026);
    final model = SyncNotices(now: () => now), a = incoming('Personal');
    model.observe([IncomingResult(a, error: 'Offline')], [a], automatic: true);
    now = now.add(const Duration(seconds: 30));
    model.dismiss();
    model.observe(
      [IncomingResult(a, error: 'Still offline')],
      [a],
      automatic: true,
    );
    expect(model.notice([a]), isNull);
    model.observe([IncomingResult(a)], [a], automatic: true);
    model.observe([IncomingResult(a, error: 'Again')], [a], automatic: true);
    now = now.add(const Duration(seconds: 30));
    expect(model.notice([a]), contains('Again'));
  });

  test(
    'queued explicit Refresh retains its origin after a held automatic check',
    () async {
      var now = DateTime.utc(2026);
      final repo = SyncRepository(), gate = Completer<List<IncomingResult>>();
      repo.held = gate;
      final workspace = Workspace(repo, MemorySettings(), syncClock: () => now);
      addTearDown(workspace.dispose);
      final background = workspace.refresh(automatic: true);
      await Future<void>.delayed(Duration.zero);
      await workspace.refresh();
      expect(workspace.notice, 'Refresh queued');
      repo.held = null;
      gate.complete([
        IncomingResult(repo.incomingAccounts.single, error: 'First'),
      ]);
      await background;
      expect(repo.checks, 2);
      expect(
        workspace.error,
        'Personal sync failed: The server is unavailable. Retry.',
      );
      workspace.clearError();
      now = now.add(const Duration(seconds: 30));
      expect(
        workspace.error,
        'Personal sync failed: The server is unavailable. Retry.',
      );
    },
  );

  test(
    'background replies preserve unrelated failure and retry ownership',
    () async {
      var now = DateTime.utc(2026);
      final repo = SyncRepository();
      final workspace = Workspace(repo, MemorySettings(), syncClock: () => now);
      addTearDown(workspace.dispose);
      var retried = false;
      void retry() {
        retried = true;
      }

      workspace.error = 'Draft save failed';
      workspace.retry = retry;
      await workspace.refresh(automatic: true);
      now = now.add(const Duration(seconds: 30));
      expect(workspace.error, 'Draft save failed');
      expect(workspace.retry, same(retry));
      workspace.retry!();
      expect(retried, isTrue);
      workspace.clearError();
      expect(workspace.error, contains('Personal sync failed'));
      repo.failure = null;
      await workspace.refresh(automatic: true);
      expect(workspace.error, isNull);
    },
  );

  test(
    'late results after reconnect, removal or foreground expiry cannot publish',
    () async {
      for (final change in ['reconnect', 'remove', 'foreground']) {
        final repo = SyncRepository(), gate = Completer<List<IncomingResult>>();
        repo.held = gate;
        final workspace = Workspace(repo, MemorySettings());
        final previous = repo.incomingAccounts.single;
        final refresh = workspace.refresh();
        await Future<void>.delayed(Duration.zero);
        if (change == 'reconnect') {
          repo.incomingAccounts = [incoming('Personal', slot: 'new')];
        }
        if (change == 'remove') await workspace.accountRemoved('Personal');
        if (change == 'foreground') workspace.setForeground(false);
        gate.complete([IncomingResult(previous, error: 'Late failure')]);
        await refresh;
        expect(workspace.error, isNull, reason: change);
        workspace.dispose();
      }
    },
  );

  testWidgets(
    'foreground timer delays notice; visible Refresh and Retry recover immediately',
    (tester) async {
      var now = DateTime.utc(2026);
      final repo = SyncRepository();
      final workspace = Workspace(repo, MemorySettings(), syncClock: () => now);
      await workspace.initialize();
      await tester.pumpWidget(ShepApp(workspace: workspace));
      now = now.add(const Duration(seconds: 15));
      await tester.pump(const Duration(seconds: 15));
      await tester.pump();
      expect(repo.checks, 1);
      expect(find.textContaining('Personal sync failed'), findsNothing);
      await tester.tap(find.byTooltip('Refresh'));
      await tester.pump();
      await tester.pump();
      expect(
        find.text('Personal sync failed: The server is unavailable. Retry.'),
        findsOneWidget,
      );
      repo.failure = null;
      await tester.tap(find.text('Retry'));
      await tester.pump();
      await tester.pump();
      expect(find.textContaining('Personal sync failed'), findsNothing);
      repo.failure = 'Still offline';
      now = now.add(const Duration(seconds: 15));
      await tester.pump(const Duration(seconds: 15));
      await tester.pump();
      now = now.add(const Duration(seconds: 30));
      await tester.pump(const Duration(seconds: 30));
      await tester.pump();
      expect(find.text('Personal sync failed: Still offline'), findsOneWidget);
      await tester.tap(find.byTooltip('Dismiss error'));
      await tester.pump();
      now = now.add(const Duration(seconds: 15));
      await tester.pump(const Duration(seconds: 15));
      await tester.pump();
      expect(find.textContaining('Personal sync failed'), findsNothing);
      workspace.dispose();
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  for (final brightness in Brightness.values) {
    testWidgets(
      'compact ${brightness.name} notice has working queued Refresh, Retry and Dismiss',
      (tester) async {
        await loadPreviewFonts();
        tester.view.physicalSize = const Size(390, 700);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        var now = DateTime.utc(2026);
        final repo = SyncRepository(), gate = Completer<List<IncomingResult>>();
        repo.held = gate;
        final workspace = Workspace(
          repo,
          MemorySettings(),
          syncClock: () => now,
        );
        workspace.preferences = workspace.preferences.copy(
          appearance: brightness == Brightness.dark
              ? ThemeMode.dark
              : ThemeMode.light,
        );
        await workspace.initialize();
        workspace.preferences = workspace.preferences.copy(
          appearance: brightness == Brightness.dark
              ? ThemeMode.dark
              : ThemeMode.light,
        );
        await tester.pumpWidget(ShepApp(workspace: workspace));
        now = now.add(const Duration(seconds: 15));
        await tester.pump(const Duration(seconds: 15));
        await tester.pump();
        expect(repo.checks, 1);
        await tester.tap(find.byTooltip('Queue refresh'));
        await tester.pump();
        expect(find.text('Refresh queued'), findsOneWidget);
        repo.held = null;
        gate.complete([
          IncomingResult(repo.incomingAccounts.single, error: 'Offline'),
        ]);
        await tester.pump();
        await tester.pump();
        await tester.pump();
        expect(repo.checks, 2);
        expect(find.textContaining('Personal sync failed'), findsOneWidget);
        await expectLater(
          find.byType(MaterialApp),
          matchesGoldenFile(
            'goldens/sync_notice_compact_${brightness.name}.png',
          ),
        );
        await tester.tap(find.byTooltip('Dismiss error'));
        await tester.pump();
        expect(find.textContaining('Personal sync failed'), findsNothing);
        now = now.add(const Duration(seconds: 30));
        await tester.pump(const Duration(seconds: 30));
        await tester.pump();
        expect(find.textContaining('Personal sync failed'), findsOneWidget);
        repo.failure = null;
        await tester.tap(find.text('Retry'));
        await tester.pump();
        await tester.pump();
        expect(find.textContaining('Personal sync failed'), findsNothing);
        workspace.dispose();
        await tester.pumpWidget(const SizedBox.shrink());
      },
    );
  }
}
