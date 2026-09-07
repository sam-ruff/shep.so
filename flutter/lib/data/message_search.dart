import 'package:flutter/foundation.dart';

class SearchHit {
  const SearchHit(this.block, this.start, this.end);
  final int block, start, end;
  factory SearchHit.fromJson(Map<String, dynamic> value) =>
      SearchHit(value['block'], value['start'], value['end']);
  Map<String, int> toJson() => {'block': block, 'start': start, 'end': end};
}

abstract interface class TextSearchRepository {
  Future<List<SearchHit>> findText(
    List<String> blocks,
    String query,
    bool matchCase,
  );
}

/// Fictional preview fallback. Production native uses shared Rust off-thread;
/// the separately hosted browser uses the same Rust implementation in WASM.
Future<List<SearchHit>> previewFind(
  List<String> blocks,
  String query,
  bool matchCase,
) => compute(_previewFind, (blocks, query, matchCase));
final _white = RegExp(
  r'^[\u0009-\u000d\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]$',
);
(String, List<int>, List<int>) _index(String text) {
  final normalized = StringBuffer(), starts = <int>[], ends = <int>[];
  var offset = 0, whitespace = false;
  for (final rune in text.runes) {
    final char = String.fromCharCode(rune);
    if (_white.hasMatch(char)) {
      if (whitespace) {
        ends[ends.length - 1] = offset + char.length;
      } else {
        normalized.write(' ');
        starts.add(offset);
        ends.add(offset + char.length);
      }
      whitespace = true;
    } else {
      normalized.write(char);
      for (var i = 0; i < char.length; i++) {
        starts.add(offset + i);
        ends.add(offset + i + 1);
      }
      whitespace = false;
    }
    offset += char.length;
  }
  return (normalized.toString(), starts, ends);
}

List<SearchHit> _previewFind((List<String>, String, bool) input) {
  final query = _index(input.$2).$1;
  if (query.isEmpty || query.replaceAll(' ', '').isEmpty) return [];
  final pattern = RegExp(
    RegExp.escape(query),
    unicode: true,
    caseSensitive: input.$3,
  );
  final hits = <SearchHit>[];
  for (var b = 0; b < input.$1.length; b++) {
    final (text, starts, ends) = _index(input.$1[b]);
    for (final match in pattern.allMatches(text)) {
      hits.add(SearchHit(b, starts[match.start], ends[match.end - 1]));
    }
  }
  return hits;
}
