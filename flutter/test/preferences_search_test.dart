import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/preferences_search.dart';
import 'package:shep_mobile/model/preferences.dart';

void main() {
  test('preview shares common accent, abbreviation and numeric cases', () {
    final fixture =
        jsonDecode(
              File(
                '../shared/preferences-search-cases.json',
              ).readAsStringSync(),
            )
            as Map;
    final entries = (fixture['entries'] as List)
        .map(
          (value) => PreferenceSearchEntry(
            target: value['target'],
            label: value['label'],
            section: value['section'],
            description: value['description'],
            synonyms: value['synonyms'],
          ),
        )
        .toList();
    for (final value in fixture['cases'] as List) {
      if (value['native_only'] == true) continue;
      final found = searchPreferences(entries, value['query']);
      expect(
        found.map(entries.indexOf).toList(),
        value['positions'],
        reason: value['query'],
      );
    }
  });
  test(
    'persisted preference fields have an explicit catalogue or gap review',
    () {
      expect(const Preferences().profileSettings().keys.toSet(), {
        'appearance', 'left_swipe', 'right_swipe', 'preview_lines',
        'sender_pictures', 'unified_inbox', 'reply_display',
        'reply_include_original',
        // The local Tooltips control remains a parity gap tracked in #41.
        'tooltips',
      });
      expect(
        mobilePreferenceEntries.map((entry) => entry.target).toSet().length,
        mobilePreferenceEntries.length,
      );
    },
  );
  test('matches every local caption, section, description and synonyms', () {
    for (final entry in mobilePreferenceEntries) {
      expect(
        searchPreferences(mobilePreferenceEntries, entry.label),
        contains(entry),
      );
      expect(
        searchPreferences(mobilePreferenceEntries, entry.section),
        contains(entry),
      );
    }
    expect(
      searchPreferences(mobilePreferenceEntries, 'avatars').single.label,
      'Sender pictures',
    );
    expect(
      searchPreferences(mobilePreferenceEntries, 'sender subject').single.label,
      'Preview lines',
    );
    expect(
      searchPreferences(mobilePreferenceEntries, 'dark').single.label,
      'Theme',
    );
    expect(
      searchPreferences(
        mobilePreferenceEntries,
        'reading original',
      ).single.label,
      'Quoted history',
    );
  });

  test(
    'ranks captions above supporting text and requires all distinct words',
    () {
      const caption = PreferenceSearchEntry(
        target: 'a',
        label: 'Cloud',
        section: 'Connections',
      );
      const synonym = PreferenceSearchEntry(
        target: 'b',
        label: 'Profiles',
        section: 'Connections',
        synonyms: 'cloud',
      );
      expect(searchPreferences([synonym, caption], 'cloud').first, caption);
      expect(
        searchPreferences(
          mobilePreferenceEntries,
          'preview preview lines',
        ).single.label,
        'Preview lines',
      );
      expect(
        searchPreferences(mobilePreferenceEntries, 'preview dark'),
        isEmpty,
      );
      expect(searchPreferences(mobilePreferenceEntries, '  '), isEmpty);
      expect(
        searchPreferences(mobilePreferenceEntries, 'nonexistent'),
        isEmpty,
      );
    },
  );

  test(
    'preview handles case, accents, abbreviations, Unicode and exact numbers',
    () {
      expect(
        searchPreferences(mobilePreferenceEntries, 'THEME').single.label,
        'Theme',
      );
      expect(
        searchPreferences(mobilePreferenceEntries, 'previ').single.label,
        'Preview lines',
      );
      expect(
        searchPreferences(mobilePreferenceEntries, 'APPEARÁNCE').single.label,
        'Theme',
      );
      expect(
        searchPreferences(mobilePreferenceEntries, '4').single.label,
        'Preview lines',
      );
      expect(searchPreferences(mobilePreferenceEntries, '42'), isEmpty);
      const entry = PreferenceSearchEntry(
        target: 'a',
        label: 'Café 123',
        section: 'Accounts',
      );
      expect(searchPreferences([entry], 'CAFÉ 123'), [entry]);
      expect(searchPreferences([entry], '12'), isEmpty);
      const profile = PreferenceSearchEntry(
        target: 'profile',
        label: 'Saved profiles',
        section: 'Connections',
      );
      expect(searchPreferences([profile], 'prf'), [profile]);
      const numeric = PreferenceSearchEntry(
        target: 'numeric',
        label: 'Office365',
        section: 'Connections',
      );
      expect(searchPreferences([numeric], 'office365'), [numeric]);
      expect(searchPreferences([numeric], 'office36'), isEmpty);
      expect(searchPreferences([numeric], 'office365x'), isEmpty);
      expect(searchPreferences(mobilePreferenceEntries, 'theem'), isEmpty);
    },
  );

  test('real catalogue words are never treated as typos of another word', () {
    const entries = [
      PreferenceSearchEntry(
        target: 'a',
        label: 'Shared profile',
        section: 'Connections',
      ),
      PreferenceSearchEntry(
        target: 'b',
        label: 'Shaped profile',
        section: 'Connections',
      ),
    ];
    expect(searchPreferences(entries, 'shared profile').single.target, 'a');
  });
  test(
    'long queries and provider labels cannot allocate a large edit matrix',
    () {
      expect(searchPreferences(mobilePreferenceEntries, 'a' * 257), isEmpty);
      final entry = PreferenceSearchEntry(
        target: 'a',
        label: 'a' * 5000,
        section: 'Connections',
      );
      expect(searchPreferences([entry], 'aaaa typo'), isEmpty);
      const unicode = PreferenceSearchEntry(
        target: 'b',
        label: 'Ελληνικά ١٢٣',
        section: 'Accounts',
      );
      expect(searchPreferences([unicode], 'ελληνικά ١٢٣'), [unicode]);
      expect(searchPreferences([unicode], '١٢'), isEmpty);
    },
  );
}
