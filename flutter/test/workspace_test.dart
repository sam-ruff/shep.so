import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/repository.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/mail_action_banner.dart';
import 'package:shep_mobile/ui/mail_activity.dart';
import 'support/preview_repository.dart';
import 'support/paged_repository.dart';
import 'support/sent_handover_repository.dart';

class HeldReadHandover extends SentHandoverRepository {
  final held = Completer<void>();
  final entered = Completer<void>();
  final calls = <String>[];
  @override
  Future<void> mutate(String id, Map<String, Object> fields) async {
    calls.add(id);
    if (calls.length == 1) {
      entered.complete();
      await held.future;
    }
    // The native cache accepts the old alias while the UI learns its canonical
    // identity; subsequent UI dispatches must already use the adopted identity.
    await super.mutate(adopted ? SentHandoverRepository.localId : id, fields);
  }
}

class MemorySettings implements SettingsStore {
  Preferences value = const Preferences();
  bool fail = false;
  @override
  Future<Preferences> read() async => value;
  @override
  Future<void> write(Preferences p) async {
    if (fail) throw StateError('disk full');
    value = p;
  }
}

class ControlledRepository extends PreviewRepository {
  ControlledRepository() : super(delay: Duration.zero);
  final jobs = <Completer<void>>[];
  @override
  Future<void> mutate(String id, Map<String, Object> fields) async {
    final job = Completer<void>();
    jobs.add(job);
    await job.future;
    await super.mutate(id, fields);
  }
}

class DurableControlledRepository extends PreviewRepository
    implements DurableMutationRepository {
  DurableControlledRepository({super.includeLineage})
    : super(delay: Duration.zero);

  final admitted = <String>[];
  final observedLineages = <String>[];
  final cancelledAdmissions = <String>[];
  final executions = <Completer<void>>[];
  Object? admissionFailure;

  @override
  Future<void> admitMutation(
    String id,
    Map<String, Object> fields,
    String actionId,
    String observedLineage,
  ) async {
    admitted.add(actionId);
    observedLineages.add(observedLineage);
    if (admissionFailure case final Object failure) throw failure;
  }

  @override
  Future<void> executeMutation(
    String id,
    Map<String, Object> fields,
    String actionId,
  ) async {
    final gate = Completer<void>();
    executions.add(gate);
    await gate.future;
    await super.mutate(id, fields);
  }

  @override
  Future<void> cancelAdmittedMutation(String actionId) async {
    cancelledAdmissions.add(actionId);
  }
}

class UncertainMoveRepository extends PreviewRepository {
  UncertainMoveRepository() : super(delay: Duration.zero);

  @override
  Future<void> mutate(String id, Map<String, Object> fields) async {
    throw const MailOperationFailure(
      'The provider result needs review.',
      committed: true,
    );
  }
}

class ActivityRepository extends PreviewRepository
    implements MailActivityRepository {
  final actions = <MailActivity>[
    MailActivity({
      'id': 'queued',
      'mail': '1',
      'status': 'queued',
      'fields': {'unread': false},
    }),
    MailActivity({
      'id': 'uncertain',
      'mail': '2',
      'status': 'uncertain',
      'fields': {'folder': 'Archive'},
      'error': 'Check Archive before retrying.',
    }),
  ];
  final resumed = <String>[];
  final cancelled = <String>[];
  @override
  Future<List<MailActivity>> mailActions({int offset = 0}) async =>
      actions.skip(offset).take(50).toList();
  @override
  Future<List<MailActivity>> runnableMailActions() async =>
      actions.where((action) => action.canResume).take(50).toList();
  @override
  Future<void> resumeMailAction(MailActivity action) async {
    resumed.add(action.id);
    actions.removeWhere((saved) => saved.id == action.id);
  }

  @override
  Future<void> cancelMailAction(String id) async {
    cancelled.add(id);
    actions.removeWhere((saved) => saved.id == id);
  }

  @override
  Future<void> undoMailAction(
    MailActivity action, {
    void Function()? onAdmitted,
  }) async {
    onAdmitted?.call();
    actions.removeWhere((saved) => saved.id == action.id);
  }

  @override
  Future<void> inspectMailAction(MailActivity action) async {
    actions.removeWhere((saved) => saved.id == action.id);
  }
}

class WaitingActivityRepository extends ActivityRepository {
  WaitingActivityRepository() {
    actions.removeWhere((action) => action.status != 'queued');
  }

  final resumeGate = Completer<void>();

  @override
  Future<void> resumeMailAction(MailActivity action) async {
    resumed.add(action.id);
    await resumeGate.future;
  }
}

class FailedReviewActivityRepository extends ActivityRepository {
  FailedReviewActivityRepository() {
    actions.removeWhere((action) => action.status != 'uncertain');
  }

  @override
  Future<void> inspectMailAction(MailActivity action) async {
    throw StateError('provider inspection unavailable');
  }
}

class HeldUndoActivityRepository extends ActivityRepository {
  HeldUndoActivityRepository() {
    actions
      ..clear()
      ..add(
        MailActivity({
          'id': 'confirmed',
          'mail': '1',
          'status': 'succeeded',
          'fields': {'unread': false},
        }),
      );
  }

  final providerGate = Completer<void>();

  @override
  Future<void> undoMailAction(
    MailActivity action, {
    void Function()? onAdmitted,
  }) async {
    actions.insert(
      0,
      MailActivity({
        'id': '${action.id}:undo',
        'mail': action.mail,
        'status': 'waiting',
        'fields': {'unread': true},
      }),
    );
    onAdmitted?.call();
    await providerGate.future;
  }
}

class PagedActivityRepository extends ActivityRepository {
  PagedActivityRepository() {
    actions
      ..clear()
      ..addAll(
        List.generate(
          51,
          (index) => MailActivity({
            'id': 'activity-$index',
            'mail': '$index',
            'status': 'succeeded',
            'fields': {'unread': false},
          }),
        ),
      );
  }

  final offsets = <int>[];

  @override
  Future<List<MailActivity>> mailActions({int offset = 0}) async {
    offsets.add(offset);
    return super.mailActions(offset: offset);
  }
}

class BuriedRunnableActivityRepository extends ActivityRepository {
  BuriedRunnableActivityRepository() {
    actions
      ..clear()
      ..addAll(
        List.generate(
          60,
          (index) => MailActivity({
            'id': 'done-$index',
            'mail': '$index',
            'status': 'succeeded',
            'fields': {'unread': false},
          }),
        ),
      )
      ..add(
        MailActivity({
          'id': 'buried-queued',
          'mail': 'queued',
          'status': 'queued',
          'fields': {'starred': true},
        }),
      );
  }
}

Future<void> tick() => Future<void>.delayed(Duration.zero);
Future<void> waitUntil(bool Function() ready) async {
  await (() async {
    while (!ready()) {
      await tick();
    }
  })().timeout(const Duration(seconds: 2));
}

void main() {
  readTrackingTests();
  test(
    'mail Activity fields discard native null placeholders before casting',
    () {
      final action = MailActivity({
        'id': 'null-fields',
        'mail': '1',
        'status': 'waiting',
        'fields': {'folder': null, 'unread': false, 'starred': null},
      });
      expect(action.fields, {'unread': false});
    },
  );
  test('startup resumes only safely queued mail activity', () async {
    final repository = ActivityRepository();
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await workspace.initialize();
    await waitUntil(() => repository.resumed.isNotEmpty);
    expect(repository.resumed, ['queued']);
    expect(workspace.mailActivityReview.single.id, 'uncertain');
  });
  test(
    'startup runnable query is independent of the first history page',
    () async {
      final repository = BuriedRunnableActivityRepository();
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await workspace.initialize();
      await waitUntil(() => repository.resumed.contains('buried-queued'));
      expect(workspace.mailActivities, hasLength(50));
    },
  );
  testWidgets('waiting activity exposes a working Cancel control', (
    tester,
  ) async {
    final repository = WaitingActivityRepository();
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await workspace.initialize();
    await tester.pumpWidget(
      MaterialApp(
        home: ListenableBuilder(
          listenable: workspace,
          builder: (_, _) => MailActionBanner(workspace: workspace),
        ),
      ),
    );
    expect(find.text('Mail change waiting to sync'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pump();
    expect(repository.cancelled, ['queued']);
    expect(find.text('Cancel'), findsNothing);
  });
  testWidgets('Activity keeps controls visible when Review fails', (
    tester,
  ) async {
    final repository = FailedReviewActivityRepository();
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await tester.pumpWidget(
      MaterialApp(home: MailActivityScreen(workspace: workspace)),
    );
    await tester.pump();
    await tester.tap(find.text('Review'));
    await tester.pump();
    await tester.pump();
    expect(
      find.textContaining('provider inspection unavailable'),
      findsOneWidget,
    );
    expect(find.text('Review'), findsOneWidget);
  });
  testWidgets('Mail Activity pages bounded history from the native owner', (
    tester,
  ) async {
    final repository = PagedActivityRepository();
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await tester.pumpWidget(
      MaterialApp(home: MailActivityScreen(workspace: workspace)),
    );
    await waitUntil(() => repository.offsets.isNotEmpty);
    await tester.pump();
    expect(repository.offsets, [0]);
    await tester.scrollUntilVisible(
      find.text('Load more'),
      500,
      scrollable: find.byType(Scrollable),
    );
    expect(find.text('Load more'), findsOneWidget);
    await tester.tap(find.text('Load more'));
    await tester.pump();
    expect(repository.offsets, [0, 50]);
    expect(find.text('Mark mail read'), findsOneWidget);
    expect(find.text('Load more'), findsNothing);
  });
  testWidgets('Undo paints its admitted inverse while the provider is held', (
    tester,
  ) async {
    final repository = HeldUndoActivityRepository();
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await workspace.initialize();
    await tester.pumpWidget(
      MaterialApp(home: MailActivityScreen(workspace: workspace)),
    );
    await tester.pump();
    expect(find.text('Undo'), findsOneWidget);
    await tester.tap(find.text('Undo'));
    await tester.pump();
    await tester.pump();
    expect(find.text('Waiting'), findsOneWidget);
    expect(find.text('Cancel'), findsOneWidget);
    expect(repository.providerGate.isCompleted, isFalse);
    repository.providerGate.complete();
    await tester.pump();
  });
  test(
    'unknown move result keeps its projection without enabling Undo',
    () async {
      final repository = UncertainMoveRepository();
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await workspace.initialize();
      final mail = workspace.mail('1')!;
      await workspace.change(mail.id, {'folder': 'Archive'});
      expect(workspace.mail(mail.id)?.folder, 'Archive');
      expect(workspace.moves.records, isEmpty);
      expect(workspace.undo, isNull);
      expect(workspace.error, contains('needs review'));
    },
  );
  test(
    'held read and newer unread survive alias adoption and dispatch in order',
    () async {
      final repo = HeldReadHandover();
      repo.message = repo.message.patch({'unread': true});
      final w = Workspace(repo, MemorySettings());
      addTearDown(w.dispose);
      await w.initialize();
      w.setForeground(false);
      w.navigate('Sent');
      await w.loadPage();
      w.beginReading(SentHandoverRepository.providerId);
      w.retainReader(SentHandoverRepository.providerId);
      final read = w.finishReading();
      await repo.entered.future;
      repo.syncGate.complete();
      await w.refresh();
      expect(
        w.mail(SentHandoverRepository.providerId)!.id,
        SentHandoverRepository.localId,
      );
      expect(w.mail(SentHandoverRepository.providerId)!.unread, false);
      final unread = w.change(SentHandoverRepository.providerId, {
        'unread': true,
      });
      expect(repo.calls, [SentHandoverRepository.providerId]);
      repo.held.complete();
      await read;
      await unread;
      await w.loadPage();
      expect(repo.calls, [
        SentHandoverRepository.providerId,
        SentHandoverRepository.localId,
      ]);
      expect(repo.message.unread, true);
      expect(w.mail(SentHandoverRepository.providerId)!.unread, true);
      expect(w.error, isNull);
    },
  );
  test(
    'cached scopes and counts stay available through pending read and move failure',
    () async {
      final repo = PagedRepository();
      final w = Workspace(repo, MemorySettings());
      addTearDown(w.dispose);
      await w.initialize();
      w.setForeground(false);
      final originalUnread = w.unreadCount;
      final archiveCount = repo.cached
          .where((m) => m.folder == 'Archive')
          .length;
      w.beginReading('1');
      final moved = w.change('1', {'folder': 'Archive'});
      await waitUntil(() => repo.jobs.length == 1);
      w.navigate('Archive');
      await w.loadPage();
      expect(w.visible.any((m) => m.id == '1'), true);
      expect(w.visible.firstWhere((m) => m.id == '1').unread, false);
      expect(w.resultCount, archiveCount + 1);
      expect(w.unreadCount, originalUnread - 1);
      expect(
        repo.jobs.length,
        1,
      ); // Read still held; Move has not reached the provider.
      w.navigate('Inbox');
      w.setFilter('Unread');
      await w.loadPage();
      expect(w.visible.any((m) => m.id == '1'), false);
      expect(w.resultCount, originalUnread - 1);
      repo.jobs[0].completeError(StateError('Read failed'));
      await waitUntil(() => repo.jobs.length == 2);
      await w.loadPage();
      expect(w.unreadCount, originalUnread - 1); // Still projected in Archive.
      expect(w.moves.label, 'Archived 1 message');
      expect(w.undo, isNotNull);
      repo.jobs[1].completeError(StateError('Move failed'));
      await moved;
      await w.loadPage();
      expect(w.visible.firstWhere((m) => m.id == '1').unread, true);
      expect(w.unreadCount, originalUnread);
      expect(w.resultCount, originalUnread);
      expect(w.error, contains('restored'));
    },
  );

  test(
    'off-page read failure restores global count without changing a new reader',
    () async {
      final repo = PagedRepository();
      final w = Workspace(repo, MemorySettings());
      addTearDown(w.dispose);
      await w.initialize();
      w.setForeground(false);
      final unread = w.unreadCount;
      w.beginReading('1');
      final read = w.finishReading();
      await waitUntil(() => repo.jobs.isNotEmpty);
      w.navigate('Sent');
      await w.loadPage();
      expect(w.unreadCount, unread - 1);
      expect(w.visible.any((m) => m.id == '1'), false);
      expect(w.folder, 'Sent');
      repo.jobs.single.completeError(StateError('Read failed'));
      await read;
      await w.loadPage();
      expect(w.folder, 'Sent');
      expect(w.unreadCount, unread);
      w.navigate('Inbox');
      await w.loadPage();
      expect(w.mail('1')!.unread, true);
    },
  );
  test(
    'paged refresh retains the metadata needed to Undo a moved row',
    () async {
      final repo = PagedRepository();
      final w = Workspace(repo, MemorySettings());
      addTearDown(w.dispose);
      await w.initialize();
      w.setForeground(false);
      final moved = w.action('1', MailAction.archive);
      await tick();
      repo.jobs[0].complete();
      await moved;
      await w.loadPage();
      expect(w.mail('1'), isNull);
      expect(w.undo, isNotNull);
      w.undo!();
      expect(w.mail('1')!.folder, 'Inbox');
      await tick();
      expect(repo.jobs.length, 2);
      repo.jobs[1].complete();
      await tick();
      await tick();
      await tick();
      expect(repo.cached.firstWhere((m) => m.id == '1').folder, 'Inbox');
      expect(w.visible.any((m) => m.id == '1'), true);
      expect(w.error, isNull);
    },
  );

  test(
    'later input is durably admitted before older provider work finishes',
    () async {
      final repo = DurableControlledRepository();
      final workspace = Workspace(repo, MemorySettings());
      final first = workspace.change('1', {'unread': false});
      await tick();
      expect(repo.admitted, hasLength(1));
      expect(repo.executions, hasLength(1));
      final second = workspace.change('1', {'starred': true});
      await tick();
      expect(repo.admitted, hasLength(2));
      expect(repo.observedLineages, ['fixture-1', 'fixture-1']);
      expect(repo.executions, hasLength(1));
      repo.executions.single.complete();
      await waitUntil(() => repo.executions.length == 2);
      expect(repo.executions, hasLength(2));
      repo.executions.last.complete();
      await Future.wait([first, second]);
    },
  );

  test(
    'failed admission restores only its fields while older work continues',
    () async {
      final repo = DurableControlledRepository();
      final workspace = Workspace(repo, MemorySettings());
      final first = workspace.change('1', {'unread': false});
      await waitUntil(() => repo.executions.length == 1);
      repo.admissionFailure = StateError('journal full');
      await workspace.change('1', {'starred': true});
      expect(workspace.mail('1')!.unread, isFalse);
      expect(workspace.mail('1')!.starred, isFalse);
      expect(workspace.error, contains('restored'));
      expect(workspace.pending, 1);
      repo.executions.single.complete();
      await first;
      await waitUntil(() => workspace.pending == 0);
    },
  );

  test(
    'failed move admission restores the row without an unhandled future',
    () async {
      final repo = DurableControlledRepository()
        ..admissionFailure = StateError('journal full');
      final workspace = Workspace(repo, MemorySettings());
      addTearDown(workspace.dispose);
      await workspace.change('1', {'folder': 'Archive'});
      await tick();
      expect(workspace.mail('1')!.folder, 'Inbox');
      expect(workspace.pending, 0);
      expect(workspace.error, contains('restored'));
      expect(repo.executions, isEmpty);
    },
  );

  test('archive removes immediately while persistence is pending', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final job = w.action('1', MailAction.archive);
    expect(w.visible.any((m) => m.id == '1'), false);
    expect(w.pending, 1);
    w.navigate('Archive');
    expect(w.visible.any((m) => m.id == '1'), true);
    await tick();
    repo.jobs.single.complete();
    await job;
    expect(w.pending, 0);
    expect(repo.cached.first.folder, 'Archive');
  });

  test(
    'durable input without an observed lineage is rejected locally',
    () async {
      final repo = DurableControlledRepository(includeLineage: false);
      final workspace = Workspace(repo, MemorySettings());

      await workspace.change('1', {'starred': true});

      expect(repo.admitted, isEmpty);
      expect(repo.executions, isEmpty);
      expect(workspace.mail('1')!.starred, isFalse);
      expect(workspace.error, contains('changed since it was shown'));
    },
  );
  test('failed move rolls back and retains browsed folder', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final job = w.action('1', MailAction.archive);
    w.navigate('Sent');
    await tick();
    repo.jobs.single.completeError(StateError('reject'));
    await job;
    expect(w.folder, 'Sent');
    expect(w.mail('1')!.folder, 'Inbox');
    expect(w.error, contains('restored'));
  });
  test('old flag failure preserves newer read and flag intent', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final first = w.action('1', MailAction.star);
    final second = w.action('1', MailAction.star);
    final third = w.action('1', MailAction.read);
    expect(w.mail('1')!.starred, false);
    expect(w.mail('1')!.unread, false);
    await tick();
    repo.jobs[0].completeError(StateError('reject'));
    await first;
    await tick();
    repo.jobs[1].complete();
    await second;
    await tick();
    repo.jobs[2].complete();
    await third;
    expect(w.mail('1')!.starred, false);
    expect(repo.cached.first.unread, false);
  });
  test('queued failures converge to confirmed state', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final jobs = [
      w.action('1', MailAction.star),
      w.action('1', MailAction.star),
      w.action('1', MailAction.star),
    ];
    for (var i = 0; i < 3; i++) {
      await tick();
      repo.jobs[i].completeError(StateError('reject'));
      await jobs[i];
    }
    expect(w.mail('1')!.starred, false);
    expect(w.error, isNotNull);
  });
  test('undo serializes behind pending archive', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final first = w.action('1', MailAction.archive);
    await waitUntil(() => repo.jobs.isNotEmpty);
    w.undo!();
    expect(w.mail('1')!.folder, 'Inbox');
    await tick();
    repo.jobs[0].complete();
    await first;
    await tick();
    repo.jobs[1].complete();
    await tick();
    await tick();
    expect(repo.cached.first.folder, 'Inbox');
  });
  test('refresh cannot restore old pending action', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final job = w.action('1', MailAction.archive);
    await w.refresh();
    expect(w.mail('1')!.folder, 'Archive');
    await tick();
    repo.jobs[0].complete();
    await job;
  });
  test('latest preferences persist together', () async {
    final settings = MemorySettings();
    final w = Workspace(PreviewRepository(), settings);
    final a = w.savePreferences(w.preferences.copy(appearance: ThemeMode.dark));
    final b = w.savePreferences(
      w.preferences.copy(
        leftSwipe: MailAction.star,
        rightSwipe: MailAction.none,
        previewLines: 0,
      ),
    );
    await Future.wait([a, b]);
    final reopened = Workspace(PreviewRepository(), settings);
    await reopened.initialize();
    expect(reopened.preferences.appearance, ThemeMode.dark);
    expect(reopened.preferences.leftSwipe, MailAction.star);
    expect(reopened.preferences.rightSwipe, MailAction.none);
    expect(reopened.preferences.previewLines, 0);
  });
  test('preference failure retries current values', () async {
    final settings = MemorySettings()..fail = true;
    final w = Workspace(PreviewRepository(), settings);
    await w.savePreferences(w.preferences.copy(appearance: ThemeMode.dark));
    expect(w.error, contains('Retry'));
    settings.fail = false;
    w.retry!();
    await tick();
    await tick();
    expect(settings.value.appearance, ThemeMode.dark);
  });
  test('preview sending refusal retains draft', () async {
    final w = Workspace(
      PreviewRepository(delay: Duration.zero),
      MemorySettings(),
    );
    const draft = Draft(
      id: 'd1',
      to: 'test@example.test',
      subject: 'Test',
      body: 'Unsent',
    );
    expect(await w.saveDraft(draft), true);
    expect(await w.send(draft), false);
    expect(w.drafts['d1']!.body, 'Unsent');
  });
  test('filter and sort recompute after flags', () async {
    final w = Workspace(
      PreviewRepository(delay: Duration.zero),
      MemorySettings(),
    );
    w.setFilter('Flagged');
    expect(w.visible.length, 1);
    await w.action('1', MailAction.star);
    expect(w.visible.length, 2);
    w.sort();
    expect(w.visible.last.id, '1');
  });
  test('all swipe settings and disabled values round trip', () {
    for (final a in MailAction.values) {
      final p = Preferences.decode(
        Preferences(leftSwipe: a, rightSwipe: a).encode(),
      );
      expect(p.leftSwipe, a);
      expect(p.rightSwipe, a);
    }
    expect(() => Preferences.decode('{"version":2}'), throwsFormatException);
  });
}

// These contracts drive the production ordering/projection model with held
// provider acknowledgments; native control/persistence equivalents are separate.
void readTrackingTests() {
  test(
    'retention and refresh are not reading; only leaving a deliberate visit reads',
    () async {
      final repo = ControlledRepository();
      final model = Workspace(repo, MemorySettings());
      addTearDown(model.dispose);
      model.retainReader('1');
      await model.refresh();
      model.navigate('Inbox');
      await tick();
      expect(repo.jobs, isEmpty);
      model.beginReading('1');
      expect(model.mail('1')!.unread, true);
      await model.refresh();
      expect(repo.jobs, isEmpty);
      model.beginReading('2');
      await tick();
      expect(model.mail('1')!.unread, false);
      repo.jobs.single.complete();
      await waitUntil(() => model.pending == 0);
      expect(repo.cached.firstWhere((m) => m.id == '1').unread, false);
    },
  );
  test(
    'explicit unread preserves newer intent after an older automatic acknowledgment',
    () async {
      final repo = ControlledRepository();
      final model = Workspace(repo, MemorySettings());
      addTearDown(model.dispose);
      model.beginReading('1');
      final read = model.finishReading();
      await tick();
      final unread = model.change('1', {'unread': true});
      model.beginReading('2');
      repo.jobs[0].complete();
      await read;
      await tick();
      expect(model.mail('1')!.unread, true);
      repo.jobs[1].complete();
      await unread;
      expect(repo.cached.first.unread, true);
    },
  );
  test(
    'move waits for automatic read; its failure preserves the new Undo and folder',
    () async {
      final repo = ControlledRepository();
      final w = Workspace(repo, MemorySettings());
      addTearDown(w.dispose);
      w.beginReading('1');
      final move = w.action('1', MailAction.archive);
      final undo = w.undo;
      expect(w.mail('1')!.folder, 'Archive');
      expect(w.mail('1')!.unread, false);
      await tick();
      expect(repo.jobs.length, 1);
      repo.jobs[0].completeError(StateError('read rejected'));
      await tick();
      expect(repo.jobs.length, 2);
      expect(w.mail('1')!.unread, true);
      expect(w.moves.label, 'Archived 1 message');
      expect(w.undo, same(undo));
      expect(w.error, contains('restored'));
      repo.jobs[1].complete();
      await move;
      expect(repo.cached.first.folder, 'Archive');
      expect(repo.cached.first.unread, true);
    },
  );
  test(
    'cache release is not reading and unchanged explicit unread cancels the visit',
    () async {
      final repo = ControlledRepository();
      final w = Workspace(repo, MemorySettings());
      addTearDown(w.dispose);
      w.retainReader('1');
      w.beginReading('1');
      w.releaseReader('1');
      expect(w.mail('1')!.unread, true);
      await w.change('1', {'unread': true});
      await w.finishReading();
      expect(repo.jobs, isEmpty);
      expect(w.mail('1')!.unread, true);
    },
  );
}
