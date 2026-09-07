import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/selection.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/mail_selection.dart';
import 'support/selection_repository.dart';

List<Mail> selectionMail([int count = 125]) => List.generate(
  count,
  (i) => Mail(
    id: 'm${i.toString().padLeft(3, '0')}',
    sender: 'Selection sender',
    address: 'sender@example.test',
    subject: 'Selection message $i',
    preview: 'Captured scope fixture',
    body: 'Body $i',
    account: 'Work',
    accountId: 'work',
    folder: 'Inbox',
    date: DateTime.utc(2026).add(Duration(seconds: count - i)),
    unread: i.isEven,
    starred: i % 3 == 0,
    attachments: [],
  ),
);
Future<void> settled(bool Function() ready) async {
  for (var i = 0; !ready(); i++) {
    if (i == 1000) fail('Selection did not settle');
    await Future<void>.delayed(Duration.zero);
  }
  await Future<void>.delayed(Duration.zero);
}

class HeldSelection implements SelectionRepository {
  HeldSelection(this.delegate);
  final PreviewSelectionRepository delegate;
  final calls = <String>[];
  Completer<void>? gate, responseGate;
  String? holdKind, rejectKind, lostKind, responseKind;
  @override
  Future<dynamic> selection(
    Map<String, Object?> command, {
    List<String> observed = const [],
  }) async {
    final kind = command['kind'] as String;
    calls.add(kind);
    if (holdKind == kind) {
      holdKind = null;
      await gate!.future;
    }
    if (rejectKind == kind) {
      rejectKind = null;
      throw StateError('Synthetic rejected selection');
    }
    final value = await delegate.selection(command, observed: observed);
    if (responseKind == kind) {
      responseKind = null;
      await responseGate!.future;
    }
    if (lostKind == kind) {
      lostKind = null;
      throw StateError('Synthetic lost selection acknowledgment');
    }
    return value;
  }
}

void main() {
  test(
    'all, page observations, cross-page ranges and Clear preserve captured scope',
    () async {
      final mail = selectionMail();
      final repo = PreviewSelectionRepository(() => mail);
      final model = MailSelection(
        repository: repo,
        scope: () => {'folder': 'Inbox'},
        currentCount: () => mail.length,
        changed: () {},
      );
      addTearDown(model.dispose);
      for (final m in mail.take(50)) {
        model.watch(m.id);
      }
      model.toggle(mail.first.id);
      expect(model.selected(mail.first.id), true);
      await settled(() => !model.pending);
      expect(model.count, 1);
      model.all();
      expect(model.count, 125);
      await settled(() => !model.pending);
      for (final m in mail.take(50)) {
        model.unwatch(m.id);
      }
      for (final m in mail.skip(100)) {
        model.watch(m.id);
      }
      await settled(() => model.selected(mail[100].id));
      model.range(mail[100].id);
      expect(model.count, 101);
      await settled(() => !model.pending);
      expect(model.count, 101);
      expect(model.selected(mail[100].id), true);
      expect(model.selected(mail[101].id), false);
      model.clear();
      expect(model.count, 0);
      expect(model.mode, true);
      await settled(() => !model.pending);
      model.done();
      await settled(() => repo.captures.isEmpty);
    },
  );
  test(
    'new arrivals stay outside selection until chosen or explicit Select all',
    () async {
      final mail = selectionMail();
      final repo = PreviewSelectionRepository(() => mail);
      final model = MailSelection(
        repository: repo,
        scope: () => {'folder': 'Inbox'},
        currentCount: () => mail.length,
        changed: () {},
      );
      addTearDown(model.dispose);
      model.watch(mail[0].id);
      model.all();
      await settled(() => !model.pending);
      final arrival = mail[0].patch({'id': 'arrival'});
      mail.add(arrival);
      model.watch(arrival.id);
      model.refresh();
      await settled(() => model.snapshot?.total == 125);
      expect(model.count, 125);
      expect(model.selected(arrival.id), false);
      model.toggle(arrival.id);
      expect(model.count, 126);
      await settled(() => !model.pending);
      expect(model.count, 126);
      expect(model.selected(arrival.id), true);
      final next = mail[0].patch({'id': 'another-arrival'});
      mail.add(next);
      model.watch(next.id);
      model.refresh();
      await Future<void>.delayed(Duration.zero);
      expect(model.selected(next.id), false);
      model.all();
      expect(model.count, 127);
      await settled(() => !model.pending);
      expect(model.selected(next.id), true);
    },
  );
  test(
    'failed earlier gesture rolls back only itself and Retry retains later input',
    () async {
      final mail = selectionMail();
      final memory = PreviewSelectionRepository(() => mail);
      final repo = HeldSelection(memory);
      final model = MailSelection(
        repository: repo,
        scope: () => {'folder': 'Inbox'},
        currentCount: () => mail.length,
        changed: () {},
      );
      addTearDown(model.dispose);
      model.watch(mail[0].id);
      model.watch(mail[1].id);
      model.all();
      await settled(() => !model.pending);
      repo.gate = Completer<void>();
      repo.holdKind = 'change';
      repo.rejectKind = 'change';
      model.toggle(mail[0].id);
      model.toggle(mail[1].id);
      expect(model.count, 123);
      repo.gate!.complete();
      await settled(() => model.error != null);
      expect(model.count, 124);
      expect(model.selected(mail[0].id), true);
      expect(model.selected(mail[1].id), false);
      model.retry();
      expect(model.count, 123);
      await settled(() => !model.pending);
      expect(model.count, 123);
      expect(model.error, isNull);
    },
  );
  test(
    'lost acknowledgments recover exactly the committed revision without replay',
    () async {
      final mail = selectionMail();
      final memory = PreviewSelectionRepository(() => mail);
      final repo = HeldSelection(memory)..lostKind = 'capture';
      final model = MailSelection(
        repository: repo,
        scope: () => {'folder': 'Inbox'},
        currentCount: () => mail.length,
        changed: () {},
      );
      addTearDown(model.dispose);
      model.watch(mail.first.id);
      model.start();
      await settled(() => !model.pending);
      expect(repo.calls.where((k) => k == 'capture').length, 1);
      expect(model.error, isNull);
      repo.lostKind = 'change';
      model.toggle(mail.first.id);
      await settled(() => !model.pending);
      expect(repo.calls.where((k) => k == 'change').length, 1);
      expect(model.count, 1);
      expect(model.error, isNull);
    },
  );
  test(
    'changing scope during capture releases the old token and never replaces the new view',
    () async {
      final mail = selectionMail();
      final memory = PreviewSelectionRepository(() => mail);
      final repo = HeldSelection(memory)
        ..holdKind = 'capture'
        ..gate = Completer<void>();
      var folder = 'Inbox';
      final model = MailSelection(
        repository: repo,
        scope: () => {'folder': folder},
        currentCount: () => mail.length,
        changed: () {},
      );
      addTearDown(model.dispose);
      model.all();
      model.done();
      folder = 'Archive';
      model.start();
      repo.gate!.complete();
      await settled(() => !model.pending);
      expect(model.snapshot!.total, 0);
      expect(model.mode, true);
      expect(memory.captures.length, 1);
      model.done();
      await settled(() => memory.captures.isEmpty);
    },
  );
  test('gesture backpressure keeps the admitted queue moving', () async {
    final mail = selectionMail();
    final memory = PreviewSelectionRepository(() => mail);
    final repo = HeldSelection(memory)
      ..holdKind = 'capture'
      ..gate = Completer<void>();
    final model = MailSelection(
      repository: repo,
      scope: () => {'folder': 'Inbox'},
      currentCount: () => mail.length,
      changed: () {},
    );
    addTearDown(model.dispose);
    for (final m in mail.take(33)) {
      model.watch(m.id);
      model.toggle(m.id);
    }
    expect(model.warning, isNotNull);
    expect(model.error, isNull);
    expect(model.count, 32);
    expect(model.anchor, mail[31].id);
    model.clear();
    expect(model.anchor, mail[31].id);
    repo.gate!.complete();
    await settled(() => !model.pending);
    expect(model.count, 32);
    expect(repo.calls.where((c) => c == 'change').length, 32);
    expect(model.selected(mail[32].id), false);
  });

  test(
    'refresh during an old observation is read again before settling',
    () async {
      final mail = selectionMail(2);
      final memory = PreviewSelectionRepository(() => mail);
      final repo = HeldSelection(memory);
      final model = MailSelection(
        repository: repo,
        scope: () => {'folder': 'Inbox'},
        currentCount: () => mail.length,
        changed: () {},
      );
      addTearDown(model.dispose);
      model.watch(mail.first.id);
      model.all();
      await settled(() => !model.pending);
      repo.responseKind = 'observe';
      repo.responseGate = Completer<void>();
      model.refresh();
      await settled(() => repo.responseKind == null);
      mail.removeAt(0);
      model.refresh();
      repo.responseGate!.complete();
      await settled(() => model.snapshot?.available == 1);
      expect(model.count, 2);
      expect(repo.calls.where((k) => k == 'observe').length, 2);
    },
  );

  test('scrolling keeps pending deselection counts and range intent', () async {
    final mail = selectionMail();
    final memory = PreviewSelectionRepository(() => mail);
    final repo = HeldSelection(memory);
    final model = MailSelection(
      repository: repo,
      scope: () => {'folder': 'Inbox'},
      currentCount: () => mail.length,
      changed: () {},
    );
    addTearDown(model.dispose);
    for (final m in mail.take(3)) {
      model.watch(m.id);
    }
    model.all();
    await settled(() => !model.pending);
    repo.gate = Completer<void>();
    repo.holdKind = 'change';
    model.toggle(mail.first.id);
    expect(model.count, 124);
    model.unwatch(mail.first.id);
    expect(model.count, 124);
    repo.gate!.complete();
    await settled(() => !model.pending);
    expect(model.count, 124);

    model.toggle(mail[1].id, clearOthers: true);
    await settled(() => !model.pending);
    repo.gate = Completer<void>();
    repo.holdKind = 'change';
    model.range(mail[2].id);
    model.toggle(mail[2].id);
    expect(model.count, 1);
    repo.gate!.complete();
    await settled(() => !model.pending);
    expect(model.count, 1);
    expect(model.selected(mail[1].id), true);
    expect(model.selected(mail[2].id), false);
  });

  test(
    'failed release is retried before another capture and disposal releases it',
    () async {
      final mail = selectionMail();
      final memory = PreviewSelectionRepository(() => mail);
      final repo = HeldSelection(memory);
      final model = MailSelection(
        repository: repo,
        scope: () => {'folder': 'Inbox'},
        currentCount: () => mail.length,
        changed: () {},
      );
      model.all();
      await settled(() => !model.pending);
      repo.rejectKind = 'release';
      model.done();
      model.start();
      await settled(() => model.error != null);
      expect(memory.captures.length, 1);
      expect(repo.calls.where((k) => k == 'capture').length, 2);
      model.retry();
      await settled(() => !model.pending);
      expect(memory.captures.length, 1);
      expect(repo.calls.where((k) => k == 'capture').length, 3);
      model.dispose();
      await settled(() => memory.captures.isEmpty);
    },
  );
}
