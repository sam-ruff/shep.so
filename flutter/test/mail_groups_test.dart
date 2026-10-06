import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/groups.dart';
import 'package:shep_mobile/model/mail_groups.dart';
import 'package:shep_mobile/model/mail_selection.dart';
import 'support/bulk_fixture.dart';
import 'support/preview_repository.dart';

Future<void> settled(bool Function() ready) async {
  for (var i = 0; !ready(); i++) {
    if (i == 5000) fail('Controllers did not settle');
    await Future<void>.delayed(const Duration(milliseconds: 1));
  }
}

class Harness {
  Harness({Duration stepDelay = Duration.zero, PreviewRepository? repository})
    : repository = repository ?? bulkPreviewRepository(stepDelay: stepDelay) {
    selection = MailSelection(
      repository: this.repository,
      scope: () => {'folder': folder},
      currentCount: () => inbox.length,
      changed: () => changes++,
    );
    groups = MailGroups(
      repository: this.repository,
      changed: () => changes++,
      refreshMail: () async => repaints++,
    );
  }
  final PreviewRepository repository;
  late final MailSelection selection;
  late final MailGroups groups;
  int changes = 0, repaints = 0;
  String folder = 'Inbox';
  List<String> get inbox => repository.cached
      .where((m) => m.folder == 'Inbox')
      .map((m) => m.id)
      .toList();

  Future<GroupJob> reviewAll(GroupAction action) async {
    for (final id in inbox.take(50)) {
      selection.watch(id);
    }
    selection.all();
    await settled(() => selection.ready);
    final review = await groups.prepare(selection, action);
    expect(review, isNotNull);
    return review!;
  }

  Future<void> finished() =>
      settled(() => !groups.running && !groups.historyLoading);
  GroupJob job(String id) => groups.jobs.firstWhere((j) => j.id == id);
  void dispose() {
    selection.dispose();
    groups.dispose();
  }
}

void main() {
  test(
    'frozen review counts per account, approval executes every step',
    () async {
      final h = Harness();
      addTearDown(h.dispose);
      final review = await h.reviewAll(GroupAction.archive);
      expect(review.total, 130);
      expect(review.state, 'review');
      expect(
        review.groups.map(
          (g) => '${g['account']}:${g['folder']}:${g['total']}',
        ),
        containsAll(['Personal:Inbox:86', 'Work:Inbox:44']),
      );
      expect(h.selection.mode, true);
      expect(h.selection.count, 130);
      // Nothing paints or runs before approval.
      expect(h.inbox.length, 130);
      expect(await h.groups.approve(), true);
      expect(h.selection.mode, false);
      expect(h.groups.review, isNull);
      expect(h.inbox, isEmpty, reason: 'approved intent paints immediately');
      await h.finished();
      final job = h.job(review.id);
      expect(job.finished, true);
      expect(job.count('done'), 130);
      expect(job.status, 'Archived 130');
      expect(
        h.repository.cached.where((m) => m.folder == 'Archive').length,
        131,
      );
      expect(h.groups.completed?.id, review.id);
      expect(h.repaints, greaterThan(0));
    },
  );

  test(
    'individual intent after approval wins and already-applied rows skip',
    () async {
      final h = Harness(stepDelay: const Duration(milliseconds: 5));
      addTearDown(h.dispose);
      final review = await h.reviewAll(GroupAction.read);
      await h.groups.approve();
      // The newest per-field choice belongs to the individual action.
      unawaited(h.repository.mutate('bulk-124', {'unread': true}));
      await h.finished();
      final job = h.job(review.id);
      expect(job.count('done'), 64);
      expect(job.count('skipped'), 66);
      final items = await h.groups.items(job, after: 100);
      final newer = items.rows.firstWhere((i) => i.mail == 'bulk-124');
      expect(newer.state, 'skipped');
      expect(newer.reason, contains('newer change'));
      expect(
        h.repository.cached.firstWhere((m) => m.id == 'bulk-124').unread,
        true,
      );
    },
  );

  test(
    'pause holds the next step and resume continues the same group',
    () async {
      final h = Harness();
      addTearDown(h.dispose);
      final review = await h.reviewAll(GroupAction.flag);
      final hold = Completer<void>(), started = Completer<void>();
      h.repository.groupPreview.hold = hold;
      h.repository.groupPreview.stepStarted = started;
      await h.groups.approve();
      await started.future;
      expect(h.groups.running, true);
      await h.groups.pause(h.job(review.id));
      expect(h.job(review.id).paused, true);
      h.repository.groupPreview.hold = null;
      hold.complete();
      await h.finished();
      final paused = h.job(review.id);
      expect(paused.paused, true);
      expect(paused.count('done'), 1);
      expect(paused.count('pending'), 130 - 1 - paused.count('skipped'));
      await h.groups.resume(paused);
      await h.finished();
      expect(h.job(review.id).finished, true);
      expect(h.job(review.id).count('pending'), 0);
    },
  );

  test('undo cancels unsent steps and restores acknowledged rows', () async {
    final h = Harness();
    addTearDown(h.dispose);
    final review = await h.reviewAll(GroupAction.delete);
    final firstHold = Completer<void>(), firstStarted = Completer<void>();
    h.repository.groupPreview.hold = firstHold;
    h.repository.groupPreview.stepStarted = firstStarted;
    await h.groups.approve();
    await firstStarted.future;
    final secondHold = Completer<void>(), secondStarted = Completer<void>();
    h.repository.groupPreview.stepStarted = secondStarted;
    h.repository.groupPreview.hold = secondHold;
    firstHold.complete();
    await secondStarted.future;
    // The second step is in flight: Undo cancels the rest and reverses it
    // once its receipt lands.
    await h.groups.undo(h.job(review.id));
    final undoing = h.job(review.id);
    expect(undoing.undo, true);
    expect(undoing.state, 'undoing');
    expect(undoing.count('cancelled'), 128);
    h.repository.groupPreview.hold = null;
    secondHold.complete();
    await h.finished();
    final job = h.job(review.id);
    expect(job.finished, true);
    expect(job.count('undone'), 2);
    expect(job.count('cancelled'), 128);
    expect(h.inbox.length, 130);
    expect(job.status, 'Undone: 2 restored');
  });

  test(
    'failed steps retry explicitly, unconfirmed steps pause and are accepted',
    () async {
      final h = Harness();
      addTearDown(h.dispose);
      h.repository.groupPreview.outcomes['bulk-003'] = 'failed';
      h.repository.groupPreview.outcomes['bulk-010'] = 'uncertain';
      final review = await h.reviewAll(GroupAction.archive);
      await h.groups.approve();
      await h.finished();
      var job = h.job(review.id);
      expect(
        job.paused,
        true,
        reason: 'an unconfirmed result pauses the group',
      );
      expect(job.count('failed'), 1);
      expect(job.count('uncertain'), 1);
      expect(job.attention, 2);
      expect(h.groups.needingReview.length, 1);
      final page = await h.groups.items(job);
      final failed = page.rows.firstWhere((i) => i.state == 'failed');
      final uncertain = page.rows.firstWhere((i) => i.state == 'uncertain');
      expect(failed.canRetry, true);
      expect(uncertain.canAccept, true);
      expect(uncertain.canRetry, false);
      await h.groups.retry(job, failed);
      await h.finished();
      job = h.job(review.id);
      expect(job.count('failed'), 0);
      expect(job.paused, true);
      await h.groups.accept(job, uncertain);
      job = h.job(review.id);
      expect(job.count('accepted'), 1);
      await h.groups.resume(job);
      await h.finished();
      job = h.job(review.id);
      expect(job.finished, true);
      expect(job.count('done'), 129);
      expect(job.count('pending'), 0);
      expect(h.groups.error, isNull);
    },
  );

  test('a missing credential stops the loop with a retryable error', () async {
    final repository = _Unavailable();
    final groups = MailGroups(
      repository: repository,
      changed: () {},
      refreshMail: () async {},
    );
    addTearDown(groups.dispose);
    await groups.pump();
    expect(groups.running, false);
    expect(groups.error, contains('credential is unavailable'));
    expect(repository.steps, 1);
    await groups.pump();
    expect(repository.steps, 2);
  });

  test(
    'declined reviews leave History and repeated approval is refused',
    () async {
      final h = Harness();
      addTearDown(h.dispose);
      final review = await h.reviewAll(GroupAction.unflag);
      await h.groups.decline();
      expect(h.groups.review, isNull);
      expect(h.selection.mode, true);
      expect(h.selection.count, 130);
      await h.groups.refreshHistory();
      expect(h.groups.jobs.where((j) => j.id == review.id), isEmpty);
      expect(
        h.repository.groupPreview.calls,
        containsAllInOrder(['prepare', 'decline']),
      );
    },
  );

  test(
    'cancel retains exact off-page membership for the next review',
    () async {
      final h = Harness();
      addTearDown(h.dispose);
      final review = await h.reviewAll(GroupAction.archive);
      final capture = h.selection.snapshot!;
      for (final id in h.inbox.take(50)) {
        h.selection.unwatch(id);
      }
      for (final id in h.inbox.skip(100)) {
        h.selection.watch(id);
      }
      await h.groups.decline(expected: review.id);
      await settled(() => h.selection.error == null && !h.selection.pending);
      h.selection.refresh();
      await settled(() => h.selection.selected(h.inbox.last));
      expect(h.selection.snapshot!.id, capture.id);
      expect(h.selection.snapshot!.revision, capture.revision);
      expect(h.selection.count, 130);
      final next = await h.groups.prepare(h.selection, GroupAction.flag);
      expect(next!.total, 130);
      expect(next.id, isNot(review.id));
    },
  );

  test('failed prepare and lost prepare replies keep the capture', () async {
    final repository = _ReviewRepository();
    final h = Harness(repository: repository);
    addTearDown(h.dispose);
    repository.failBefore = 'prepare';
    h.selection.all();
    await settled(() => h.selection.ready);
    final capture = h.selection.snapshot!;
    expect(await h.groups.prepare(h.selection, GroupAction.archive), isNull);
    expect(h.selection.snapshot!.id, capture.id);
    expect(repository.groupPreview.jobs, isEmpty);
    repository.loseReply = 'prepare';
    final review = await h.groups.prepare(h.selection, GroupAction.archive);
    expect(review!.total, 130);
    expect(h.selection.snapshot!.id, capture.id);
    expect(
      repository.groupPreview.calls.where((c) => c == 'prepare').length,
      1,
    );
  });

  test(
    'obsolete prepare is retired without replacing a newer selection',
    () async {
      final repository = _ReviewRepository();
      final h = Harness(repository: repository);
      addTearDown(h.dispose);
      h.selection.all();
      await settled(() => h.selection.ready);
      repository.holdKind = 'prepare';
      repository.hold = Completer<void>();
      final preparing = h.groups.prepare(h.selection, GroupAction.archive);
      await repository.started.future;
      h.selection.done();
      h.folder = 'Archive';
      h.selection.toggle('6');
      await settled(() => h.selection.ready);
      final newer = h.selection.snapshot!.id;
      repository.hold!.complete();
      expect(await preparing, isNull);
      expect(h.groups.review, isNull);
      expect(repository.groupPreview.jobs, isEmpty);
      expect(h.selection.snapshot!.id, newer);
      expect(h.selection.count, 1);
    },
  );

  test('new gestures fence prepare before their native reply', () async {
    final repository = _ReviewRepository();
    final h = Harness(repository: repository);
    addTearDown(h.dispose);
    h.selection.all();
    await settled(() => h.selection.ready);
    repository.holdKind = 'prepare';
    repository.hold = Completer<void>();
    final preparing = h.groups.prepare(h.selection, GroupAction.archive);
    await repository.started.future;
    h.selection.clear();
    repository.hold!.complete();
    expect(await preparing, isNull);
    await settled(() => !h.selection.pending);
    expect(h.selection.count, 0);
    expect(h.selection.mode, true);
    expect(repository.groupPreview.jobs, isEmpty);
  });

  test('failed approval retains review and selection for retry', () async {
    final repository = _ReviewRepository();
    final h = Harness(repository: repository);
    addTearDown(h.dispose);
    final review = await h.reviewAll(GroupAction.archive);
    repository.failBefore = 'approve';
    expect(await h.groups.approve(expected: review.id), false);
    expect(h.groups.review!.id, review.id);
    expect(h.selection.count, 130);
    expect(h.selection.mode, true);
    expect(h.groups.error, contains('Could not confirm'));
    expect(await h.groups.approve(expected: review.id), true);
    expect(h.selection.mode, false);
    await h.finished();
  });

  test('lost approval inspects saved status without approving twice', () async {
    final repository = _ReviewRepository();
    final h = Harness(repository: repository);
    addTearDown(h.dispose);
    final review = await h.reviewAll(GroupAction.archive);
    repository.loseReply = 'approve';
    repository.failBefore = 'inspect';
    expect(await h.groups.approve(expected: review.id), false);
    expect(h.groups.review!.id, review.id);
    expect(h.selection.mode, true);
    expect(await h.groups.approve(expected: review.id), true);
    await h.finished();
    expect(
      repository.groupPreview.calls.where((c) => c == 'approve').length,
      1,
    );
    expect(h.selection.mode, false);
  });

  test('delayed approval never releases a replacement selection', () async {
    final repository = _ReviewRepository();
    final h = Harness(repository: repository);
    addTearDown(h.dispose);
    final review = await h.reviewAll(GroupAction.flag);
    repository.holdKind = 'approve';
    repository.hold = Completer<void>();
    final approving = h.groups.approve(expected: review.id);
    await repository.started.future;
    expect(await h.groups.decline(expected: review.id), false);
    h.selection.done();
    h.selection.toggle(h.inbox.last);
    await settled(() => h.selection.ready);
    final newer = h.selection.snapshot!.id;
    repository.hold!.complete();
    expect(await approving, true);
    expect(h.selection.snapshot!.id, newer);
    expect(h.selection.count, 1);
    await h.finished();
  });

  test(
    'failed decline stays reviewable and stale callbacks do not retarget',
    () async {
      final repository = _ReviewRepository();
      final h = Harness(repository: repository);
      addTearDown(h.dispose);
      final old = await h.reviewAll(GroupAction.archive);
      repository.failBefore = 'decline';
      expect(await h.groups.decline(expected: old.id), false);
      expect(h.groups.review!.id, old.id);
      expect(h.selection.count, 130);
      expect(await h.groups.prepare(h.selection, GroupAction.flag), isNull);
      expect(await h.groups.decline(expected: old.id), true);
      final newer = await h.groups.prepare(h.selection, GroupAction.flag);
      expect(await h.groups.decline(expected: old.id), false);
      expect(await h.groups.approve(expected: old.id), false);
      expect(h.groups.review!.id, newer!.id);
    },
  );

  test(
    'failed obsolete-review cleanup is retried before another prepare',
    () async {
      final repository = _ReviewRepository();
      final h = Harness(repository: repository);
      addTearDown(h.dispose);
      h.selection.all();
      await settled(() => h.selection.ready);
      repository.holdKind = 'prepare';
      repository.hold = Completer<void>();
      final preparing = h.groups.prepare(h.selection, GroupAction.archive);
      await repository.started.future;
      h.selection.clear();
      await settled(() => !h.selection.pending);
      repository.failBefore = 'decline';
      repository.hold!.complete();
      expect(await preparing, isNull);
      expect(repository.groupPreview.jobs.length, 1);
      expect(h.groups.error, contains('Could not close'));
    await h.groups.retryPending();
      expect(repository.groupPreview.jobs, isEmpty);
      expect(h.groups.error, isNull);
      expect(h.selection.mode, true);
      expect(h.selection.count, 0);
      h.selection.toggle(h.inbox.last);
      await settled(() => h.selection.ready);
      final next = await h.groups.prepare(h.selection, GroupAction.flag);
      expect(next!.total, 1);
      expect(repository.groupPreview.jobs.length, 1);
      expect(repository.groupPreview.jobs.keys.single, next.id);
    },
  );

  test('disposal retires the displayed unapproved review', () async {
    final h = Harness();
    await h.reviewAll(GroupAction.archive);
    h.dispose();
    await settled(() => h.repository.groupPreview.jobs.isEmpty);
    expect(h.repository.groupPreview.calls, isNot(contains('approve')));
  });

  test(
    'dispose retires a delayed prepare without notifying or approving',
    () async {
      final repository = _ReviewRepository();
      final h = Harness(repository: repository);
      h.selection.all();
      await settled(() => h.selection.ready);
      repository.holdKind = 'prepare';
      repository.hold = Completer<void>();
      final preparing = h.groups.prepare(h.selection, GroupAction.archive);
      await repository.started.future;
      h.dispose();
      final changes = h.changes;
      repository.hold!.complete();
      expect(await preparing, isNull);
      expect(h.changes, changes);
      expect(repository.groupPreview.jobs, isEmpty);
    },
  );
}

class _ReviewRepository extends PreviewRepository {
  _ReviewRepository() : super(delay: Duration.zero, extra: bulkFixtureMail());
  String? failBefore, loseReply, holdKind;
  Completer<void>? hold;
  final started = Completer<void>();

  @override
  Future<dynamic> groups(Map<String, Object?> command) async {
    final kind = command['kind'];
    if (failBefore == kind) {
      failBefore = null;
      throw StateError('Saved review fixture failure');
    }
    final result = await super.groups(command);
    if (holdKind == kind && hold != null) {
      if (!started.isCompleted) started.complete();
      await hold!.future;
    }
    if (loseReply == kind) {
      loseReply = null;
      throw StateError('Lost saved review reply');
    }
    return result;
  }
}

class _Unavailable implements GroupRepository {
  int steps = 0;
  @override
  Future<dynamic> groups(Map<String, Object?> command) async => {
    'jobs': const [],
    'runnable': false,
  };
  @override
  Future<Map<String, dynamic>> groupStep() async {
    steps++;
    return {'requires_credentials': 'fixture'};
  }
}
