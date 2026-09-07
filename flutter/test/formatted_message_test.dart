import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/formatted_message.dart';
import 'package:shep_mobile/model/formatted_message.dart';
import 'package:shep_mobile/model/message_find.dart';
import 'package:shep_mobile/data/message_search.dart';

class Repository implements FormattedMessageRepository {
  final requests = <Completer<PreparedMessage>>[];
  @override
  Future<PreparedMessage> formattedMessage(
    String id, {
    required String generation,
    required bool dark,
    required bool quotes,
  }) {
    final request = Completer<PreparedMessage>();
    requests.add(request);
    return request.future;
  }
}

const prepared = PreparedMessage(
  signature: 'synthetic',
  text: 'Plain',
  document: 'confined fixture',
);
void main() {
  test(
    'obsolete preparation success/error and disposed results cannot replace current mail',
    () async {
      final repository = Repository();
      final reader = FormattedMessage(repository, 'message');
      final first = reader.load(dark: false, quotes: false),
          old = reader.generation;
      final second = reader.load(dark: true, quotes: true),
          current = reader.generation;
      expect(old, isNot(current));
      repository.requests[0].completeError(
        StateError('Obsolete formatting failure'),
      );
      await first;
      expect(reader.loading, isTrue);
      expect(reader.error, isNull);
      repository.requests[1].complete(prepared);
      await second;
      expect(reader.prepared, prepared);
      expect(
        reader.receive({
          'type': 'content',
          'generation': old,
          'layout': 99,
          'blocks': ['old'],
        }),
        isFalse,
      );
      reader.receive({
        'type': 'content',
        'generation': current,
        'layout': 2,
        'blocks': ['new'],
        'hasQuotes': true,
      });
      reader.receive({
        'type': 'content',
        'generation': current,
        'layout': 1,
        'blocks': ['late'],
      });
      expect(reader.blocks, ['new']);
      expect(reader.hasQuotes, isTrue);
      final third = reader.load(dark: false, quotes: false);
      reader.dispose();
      repository.requests[2].complete(prepared);
      await third;
      expect(reader.prepared, isNull);
    },
  );
  test(
    'ready/configuration and highlights use current generation/layout without resize jumps',
    () async {
      final repository = Repository();
      final model = FormattedMessage(repository, 'message');
      final load = model.load(dark: false, quotes: false);
      repository.requests.single.complete(prepared);
      await load;
      final commands = <Map<String, Object?>>[];
      final subscription = model.commands.stream.listen(commands.add);
      model.configure(dark: true, quotes: true);
      expect(commands, isEmpty);
      model.receive({'type': 'ready', 'generation': model.generation});
      expect(commands.single, containsPair('dark', true));
      expect(commands.single, containsPair('quotes', true));
      model.receive({
        'type': 'content',
        'generation': model.generation,
        'layout': 1,
        'blocks': ['Alpha across spans'],
      });
      final find = MessageFind(previewFind, debounce: Duration.zero);
      find.setSource('message', model.blocks);
      find.show();
      find.setQuery('Alpha');
      while (find.pending) {
        await Future<void>.delayed(const Duration(milliseconds: 1));
      }
      expect(model.highlight(find), isTrue);
      expect(commands.last, containsPair('layout', 1));
      expect(
        commands.last,
        containsPair('hits', [
          {'block': 0, 'start': 0, 'end': 5},
        ]),
      );
      model.receive({
        'type': 'content',
        'generation': model.generation,
        'layout': 2,
        'blocks': ['Alpha across spans'],
      });
      expect(model.highlight(find), isFalse);
      expect(commands.last, containsPair('jump', false));
      model.setPlain(true);
      find.setQuery('Plain');
      expect(model.highlight(find), isFalse);
      model.displayError('old');
      expect(model.error, isNull);
      model.displayError(model.generation);
      expect(model.html, isFalse);
      expect(model.error, contains('retry'));
      await subscription.cancel();
      find.dispose();
      model.dispose();
    },
  );
}
