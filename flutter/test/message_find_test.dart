import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/message_search.dart';
import 'package:shep_mobile/model/message_find.dart';

void main() {
  test(
    'preview matches the shared desktop literal Unicode and wrapped whitespace contract',
    () async {
      final cases =
          jsonDecode(await File('../shared/find-cases.json').readAsString())
              as List;
      for (final c in cases) {
        expect(
          (await previewFind(
            List<String>.from(c['blocks']),
            c['query'],
            c['match_case'],
          )).map((h) => h.toJson()).toList(),
          c['hits'],
        );
      }
    },
  );
  test(
    'pending work coalesces and stale source/query/errors cannot restore a closed search',
    () async {
      final jobs = <(List<String>, String, Completer<List<SearchHit>>)>[];
      final find = MessageFind((blocks, query, _) {
        final c = Completer<List<SearchHit>>();
        jobs.add((blocks, query, c));
        return c.future;
      }, debounce: Duration.zero);
      addTearDown(find.dispose);
      Future<void> pump() => Future<void>.delayed(Duration.zero);
      find.setSource('first', ['old']);
      find.show();
      find.setQuery('old');
      await pump();
      expect(jobs, hasLength(1));
      find.setSource('second', ['latest']);
      find.setQuery('discarded');
      find.setQuery('latest');
      await pump();
      expect(jobs, hasLength(1));
      jobs[0].$3.completeError(StateError('obsolete'));
      await pump();
      expect(find.error, isNull);
      expect(jobs, hasLength(2));
      expect(jobs[1].$1, ['latest']);
      expect(jobs[1].$2, 'latest');
      jobs[1].$3.complete([const SearchHit(0, 0, 2), const SearchHit(0, 2, 4)]);
      await pump();
      expect(find.status, '1 of 2');
      find.next(true);
      expect(find.status, '2 of 2');
      find.next();
      expect(find.status, '1 of 2');
      find.toggleCase();
      await pump();
      find.close();
      jobs[2].$3.complete([const SearchHit(0, 0, 6)]);
      await pump();
      expect(find.hits, isEmpty);
      expect(find.open, false);
      find.show();
      await pump();
      jobs[3].$3.completeError(StateError('current'));
      await pump();
      expect(find.error, contains('Retry Find'));
      find.retry();
      await pump();
      jobs[4].$3.complete([]);
      await pump();
      expect(find.error, isNull);
      expect(find.status, 'No matches');
    },
  );
}
