import 'dart:convert';
import 'package:diacritic/diacritic.dart';

abstract interface class PreferenceSearchMatcher {
  Future<List<PreferenceSearchEntry>> match(
    List<PreferenceSearchEntry> entries,
    String query,
  );
}

class PreviewPreferenceSearchMatcher implements PreferenceSearchMatcher {
  const PreviewPreferenceSearchMatcher();
  @override
  Future<List<PreferenceSearchEntry>> match(
    List<PreferenceSearchEntry> entries,
    String query,
  ) async => searchPreferences(entries, query);
}

String encodePreferenceCatalogue(Iterable<PreferenceSearchEntry> entries) =>
    jsonEncode([
      for (final entry in entries)
        {
          'target': entry.target,
          'label': entry.label,
          'section': entry.section,
          'description': entry.description,
          'synonyms': entry.synonyms,
        },
    ]);

class PreferenceSearchEntry {
  const PreferenceSearchEntry({
    required this.target,
    required this.label,
    required this.section,
    this.description = '',
    this.synonyms = '',
  });

  final String target;
  final String label;
  final String section;
  final String description;
  final String synonyms;

  @override
  bool operator ==(Object other) =>
      other is PreferenceSearchEntry &&
      target == other.target &&
      label == other.label &&
      section == other.section &&
      description == other.description &&
      synonyms == other.synonyms;

  @override
  int get hashCode =>
      Object.hash(target, label, section, description, synonyms);
}

const mobilePreferenceEntries = [
  PreferenceSearchEntry(
    target: 'preference-theme',
    label: 'Theme',
    section: 'Appearance',
    synonyms: 'light dark system colour color brightness',
  ),
  PreferenceSearchEntry(
    target: 'preference-swipe-left',
    label: 'Swipe left',
    section: 'Swipe actions',
    description: 'Swipe a message to act. Every action is also in its menu.',
    synonyms: 'gesture archive delete read unread flag move',
  ),
  PreferenceSearchEntry(
    target: 'preference-swipe-right',
    label: 'Swipe right',
    section: 'Swipe actions',
    description: 'Swipe a message to act. Every action is also in its menu.',
    synonyms: 'gesture archive delete read unread flag move',
  ),
  PreferenceSearchEntry(
    target: 'preference-preview',
    label: 'Preview lines',
    section: 'Message list',
    description: 'Sender and subject are always shown',
    synonyms: 'snippet summary 0 1 2 3 4',
  ),
  PreferenceSearchEntry(
    target: 'preference-avatars',
    label: 'Sender pictures',
    section: 'Message list',
    synonyms: 'avatars photos contacts',
  ),
  PreferenceSearchEntry(
    target: 'preference-unified',
    label: 'Unified inbox',
    section: 'Message list',
    synonyms: 'combined all accounts',
  ),
  PreferenceSearchEntry(
    target: 'preference-quotes',
    label: 'Quoted history',
    section: 'Reading',
    synonyms:
        'conversation thread collapsed expanded latest only original message',
  ),
  PreferenceSearchEntry(
    target: 'preference-images',
    label: 'External images blocked',
    section: 'Reading',
    description: 'Remote images stay blocked; inline images can still appear.',
    synonyms: 'privacy remote pictures tracking security',
  ),
];

List<PreferenceSearchEntry> searchPreferences(
  Iterable<PreferenceSearchEntry> entries,
  String query,
) {
  if (query.runes.take(257).length > 256) return [];
  final terms = _words(query).toSet();
  if (terms.isEmpty) return [];
  final catalogue = entries.toList();
  final scored = <(PreferenceSearchEntry, int)>[];
  for (final entry in catalogue) {
    final fields = [
      entry.label,
      entry.section,
      entry.description,
      entry.synonyms,
    ];
    var score = 0;
    var matches = true;
    for (final term in terms) {
      var best = 0;
      for (var index = 0; index < fields.length; index++) {
        for (final word in _words(fields[index])) {
          final weight = [100, 40, 20, 10][index];
          if (word == term) {
            if (weight > best) best = weight;
          } else if (!RegExp(r'\p{N}', unicode: true).hasMatch(term) &&
              word.startsWith(term)) {
            if (weight ~/ 2 > best) best = weight ~/ 2;
          } else if (!RegExp(r'\p{N}', unicode: true).hasMatch(term) &&
              term.length <= 64 &&
              word.length <= 64 &&
              _subsequence(term, word)) {
            if (weight ~/ 4 > best) best = weight ~/ 4;
          }
        }
      }
      if (best == 0) {
        matches = false;
        break;
      }
      score += best;
    }
    if (matches) {
      if (_normalise(entry.label) == _normalise(query.trim())) score += 500;
      scored.add((entry, score));
    }
  }
  scored.sort((a, b) {
    final score = b.$2.compareTo(a.$2);
    return score != 0 ? score : a.$1.label.compareTo(b.$1.label);
  });
  return scored.map((item) => item.$1).toList();
}

List<String> _words(String text) => _normalise(text)
    .split(RegExp(r'[^\p{L}\p{N}]+', unicode: true))
    .where((word) => word.isNotEmpty)
    .toList();

String _normalise(String text) => removeDiacritics(text).toLowerCase();

bool _subsequence(String term, String word) {
  final chars = word.runes.iterator;
  for (final wanted in term.runes) {
    var found = false;
    while (chars.moveNext()) {
      if (chars.current == wanted) {
        found = true;
        break;
      }
    }
    if (!found) return false;
  }
  return true;
}
