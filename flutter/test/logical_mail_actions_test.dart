import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/repository.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'support/preview_repository.dart';
import 'workspace_test.dart' show MemorySettings;

class LogicalRepository extends PreviewRepository
    implements LogicalMutationRepository {
  LogicalRepository() : super(delay: Duration.zero);
  final requests = <(String, String, String)>[];
  final admittedFields = <Map<String, Object>>[];
  final cancelled = <String>[];
  Completer<void>? hold;
  bool waiting = false;
  bool unchanged = false;
  String physical = 'Corbeille';
  @override
  Future<void> admitLogicalMutation(
    String id,
    Map<String, Object> fields,
    String action,
    String lineage,
    String role,
  ) async {
    requests.add(('admit', action, role));
    admittedFields.add(Map.of(fields));
  }

  @override
  Future<Map<String, Object>> executeLogicalMutation(
    String id,
    Map<String, Object> fields,
    String action,
    String role,
  ) async {
    requests.add(('execute', action, role));
    await hold?.future;
    if (waiting) {
      throw const MailOperationFailure(
        'Destination CREATE needs review',
        committed: false,
        pending: true,
      );
    }
    final actual = <String, Object>{'folder': physical};
    if (unchanged) {
      throw MailOperationFailure(
        'Already in destination',
        unchanged: true,
        appliedFields: actual,
      );
    }
    await super.mutate(id, actual);
    return actual;
  }

  @override
  Future<void> admitMutation(
    String id,
    Map<String, Object> fields,
    String action,
    String lineage,
  ) async {
    requests.add((
      'physical-admit',
      action,
      fields['folder'] as String? ?? 'flags',
    ));
  }

  @override
  Future<void> executeMutation(
    String id,
    Map<String, Object> fields,
    String action,
  ) => super.mutate(id, fields);
  @override
  Future<void> cancelAdmittedMutation(String id) async {
    cancelled.add(id);
  }
}

void main() {
  Future<Workspace> start(LogicalRepository repository) async {
    final workspace = Workspace(repository, MemorySettings());
    await workspace.initialize();
    workspace.setForeground(false);
    addTearDown(workspace.dispose);
    return workspace;
  }

  test('logical action confirms only its actual physical target', () async {
    final repository = LogicalRepository();
    final workspace = await start(repository);
    final mail = workspace.visible.first;
    await workspace.action(mail.id, MailAction.trash);
    expect(repository.requests.map((r) => r.$1), ['admit', 'execute']);
    expect(repository.requests.map((r) => r.$3), ['trash', 'trash']);
    expect(repository.requests[0].$2, repository.requests[1].$2);
    expect(workspace.mail(mail.id)!.folder, 'Corbeille');
    expect(workspace.moves.records.single.committed, true);
    expect(workspace.moves.label, 'Deleted 1 message');
  });

  test(
    'CREATE Waiting remains pending and Undo cancels the original admission',
    () async {
      final repository = LogicalRepository()..waiting = true;
      final workspace = await start(repository);
      final mail = workspace.visible.first;
      await workspace.action(mail.id, MailAction.archive);
      final move = workspace.moves.records.single;
      expect(move.committed, false);
      expect(move.pending, true);
      expect(move.started, false);
      expect(workspace.moves.label, 'Archiving 1 message');
      expect(workspace.moves.canUndo, true);
      workspace.undoMoves(workspace.moves.records);
      for (var n = 0; repository.cancelled.isEmpty; n++) {
        if (n == 100) fail('pending Undo did not cancel');
        await Future<void>.delayed(Duration.zero);
      }
      expect(repository.cancelled, [move.actionId]);
      expect(
        repository.requests.where((r) => r.$1 == 'physical-admit'),
        isEmpty,
      );
      expect(
        repository.cached.firstWhere((m) => m.id == mail.id).folder,
        'Inbox',
      );
    },
  );

  test(
    'rejected resolved physical destination retries as a fresh logical request',
    () async {
      final repository = LogicalRepository();
      final workspace = await start(repository);
      final mail = workspace.visible.first;
      final rejected = MailActivity({
        'id': 'old-rejected',
        'mail': mail.id,
        'account': mail.accountId,
        'status': 'rejected',
        'fields': {'folder': 'INBOX.Junk Mail'},
        'logical_role': 'spam',
      });
      await workspace.retryMailActivity(rejected);
      expect(repository.requests.map((r) => r.$3), ['spam', 'spam']);
      expect(repository.requests.any((r) => r.$1 == 'physical-admit'), false);
      expect(repository.requests.first.$2, isNot('old-rejected'));
      expect(repository.admittedFields.single, {'folder': 'Spam'});
      expect(rejected.fields, {'folder': 'INBOX.Junk Mail'});
    },
  );

  test(
    'resolved unchanged action removes only its Undo and confirms physical folder',
    () async {
      final repository = LogicalRepository()
        ..unchanged = true
        ..physical = 'Inbox';
      final workspace = await start(repository);
      final mail = workspace.visible.first;
      await workspace.action(mail.id, MailAction.trash);
      expect(workspace.mail(mail.id)!.folder, 'Inbox');
      expect(workspace.moves.records, isEmpty);
      expect(workspace.undo, isNull);
      expect(workspace.error, isNull);
      expect(
        repository.requests.where((r) => r.$1 == 'physical-admit'),
        isEmpty,
      );
    },
  );
  test(
    'late resolved physical folder cannot replace newer local input',
    () async {
      final repository = LogicalRepository()..hold = Completer<void>();
      final workspace = await start(repository);
      final mail = workspace.visible.first;
      final older = workspace.action(mail.id, MailAction.trash);
      await Future<void>.delayed(Duration.zero);
      final newer = workspace.change(mail.id, {'folder': 'Projects'});
      expect(workspace.mail(mail.id)!.folder, 'Projects');
      repository.hold!.complete();
      await older;
      await newer;
      expect(workspace.mail(mail.id)!.folder, 'Projects');
    },
  );
}
