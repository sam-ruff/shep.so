import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/printing.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'support/preview_repository.dart';
import 'workspace_test.dart' show MemorySettings;

class PrintSource extends PreviewRepository implements PrintRepository {
  final requests =
      <
        ({
          String id,
          String generation,
          bool plain,
          Completer<PreparedPrint> result,
        })
      >[];
  @override
  Future<PreparedPrint> preparePrint(
    String id, {
    required String generation,
    required bool plain,
  }) {
    final result = Completer<PreparedPrint>();
    requests.add((
      id: id,
      generation: generation,
      plain: plain,
      result: result,
    ));
    return result.future;
  }
}

class Printer implements MessagePrinter {
  final opened = <({PreparedPrint value, String generation})>[];
  bool fail = false;
  @override
  Future<void> open(
    PreparedPrint prepared, {
    required String generation,
  }) async {
    if (fail) throw StateError('Printer unavailable. Retry.');
    opened.add((value: prepared, generation: generation));
  }
}

const prepared = PreparedPrint(
  document: 'Complete original source',
  title: 'Original',
  signature: 'original',
  accountId: 'work',
);
void main() {
  test(
    'pending print captures source and mode without blocking a new reader or replacing errors',
    () async {
      final repo = PrintSource(), printer = Printer();
      final w = Workspace(repo, MemorySettings(), printer: printer);
      addTearDown(w.dispose);
      final job = w.printMessage('1', plain: true);
      expect(w.isPrinting('1'), true);
      await w.printMessage('1', plain: false);
      expect(repo.requests.length, 1);
      expect(repo.requests.single.plain, true);
      w.retainReader('2');
      w.error = 'A newer independent error';
      repo.requests.single.result.complete(prepared);
      await job;
      expect(printer.opened.single.value, same(prepared));
      expect(printer.opened.single.generation, repo.requests.single.generation);
      expect(w.mail('2')?.id, '2');
      expect(w.error, 'A newer independent error');
      expect(w.isPrinting('1'), false);
    },
  );
  test(
    'preparation and native dialog failures recover with fresh generations; disposal cancels launch',
    () async {
      final repo = PrintSource(), printer = Printer();
      final w = Workspace(repo, MemorySettings(), printer: printer);
      var job = w.printMessage('1', plain: false);
      repo.requests.last.result.completeError(
        StateError('Use plain text or retry.'),
      );
      await job;
      expect(w.error, contains('plain text'));
      printer.fail = true;
      job = w.printMessage('1', plain: true);
      repo.requests.last.result.complete(prepared);
      await job;
      expect(w.error, contains('Printer unavailable'));
      expect(repo.requests[0].generation, isNot(repo.requests[1].generation));
      printer.fail = false;
      job = w.printMessage('1', plain: true);
      repo.requests.last.result.complete(prepared);
      await job;
      expect(w.error, isNull);
      expect(printer.opened.length, 1);
      job = w.printMessage('2', plain: true);
      w.dispose();
      repo.requests.last.result.complete(prepared);
      await job;
      expect(printer.opened.length, 1);
    },
  );
}
