import '../model/preferences_search.dart';
import 'package:flutter/foundation.dart';
import 'native_repository.dart';

class NativePreferenceSearchMatcher implements PreferenceSearchMatcher {
  const NativePreferenceSearchMatcher(this.repository);
  final NativeRepository repository;
  @override
  Future<List<PreferenceSearchEntry>> match(
    List<PreferenceSearchEntry> entries,
    String query,
  ) async {
    final catalogue = await compute(encodePreferenceCatalogue, entries);
    final positions =
        await repository.callBackground({
              'op': 'search_preferences',
              'catalogue': catalogue,
              'query': query,
            })
            as List;
    return [for (final position in positions.cast<int>()) entries[position]];
  }
}
