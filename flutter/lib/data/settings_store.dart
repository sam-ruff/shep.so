import 'dart:convert';
import 'package:shared_preferences/shared_preferences.dart';
import '../model/preferences.dart';
import 'profile_settings.dart';

abstract interface class SettingsStore {
  Future<Preferences> read();
  Future<void> write(Preferences value);
}

/// Object-scoped storage boundary for persistence and lost-reply tests.
abstract interface class PreferenceStorage {
  Future<String?> read();
  Future<void> write(String value);
}

class _PreferenceStorage implements PreferenceStorage {
  final store = SharedPreferencesAsync();
  @override
  Future<String?> read() => store.getString('shep.preferences.v1');
  @override
  Future<void> write(String value) =>
      store.setString('shep.preferences.v1', value);
}

/// Non-secret appearance/interaction preferences only; never mail or credentials.
class _PreferenceWrites {
  Future<void>? tail;
  Future<T> run<T>(Future<T> Function() action) {
    final result = tail == null
        ? Future<T>.sync(action)
        : tail!.then((_) => action());
    tail = result.then<void>((_) {}, onError: (Object _, StackTrace _) {});
    return result;
  }
}

class DeviceSettings implements SettingsStore, ProfileSettingsStore {
  DeviceSettings({PreferenceStorage? storage})
    : _storage = storage ?? _PreferenceStorage(),
      _writes = storage == null
          ? _deviceWrites
          : (_owners[storage] ??= _PreferenceWrites());
  final PreferenceStorage _storage;
  final _PreferenceWrites _writes;
  // All platform handles share their one preference key. Injected independent
  // storage has its own owner; two handles for the same storage still serialize.
  static final _deviceWrites = _PreferenceWrites();
  static final _owners = Expando<_PreferenceWrites>();
  Future<T> _ordered<T>(Future<T> Function() action) => _writes.run(action);

  Future<_Ledger> _read() async => _Ledger.decode(await _storage.read());
  @override
  Future<Preferences> read() =>
      _ordered(() async => (await _read()).preferences);
  @override
  Future<void> write(Preferences value) => _ordered(() async {
    final current = await _read();
    await _storage.write(current.update(value).encode());
  });
  @override
  Future<Preferences> saveLocal(Map<String, Object?> changes) {
    final selected = Map<String, Object?>.unmodifiable(changes);
    return _ordered(() async {
      final current = await _read();
      final next = current.update(
        current.preferences.applyProfile(selected),
        intent: selected.keys.toSet(),
      );
      await _storage.write(next.encode());
      return next.preferences;
    });
  }

  @override
  Future<ProfileSettingsSnapshot> profileSnapshot() => _ordered(() async {
    final current = await _read();
    return ProfileSettingsSnapshot(
      current.preferences,
      Map.unmodifiable(current.revisions),
    );
  });
  @override
  Future<ProfileSettingsReceipt> applyProfile({
    required String id,
    required ProfileSettingsSnapshot baseline,
    required Map<String, Object?> changes,
  }) {
    final selected = Map<String, Object?>.unmodifiable(changes);
    return _ordered(() async {
      if (id.isEmpty || id.length > 128) {
        throw const FormatException('Invalid settings review identity.');
      }
      // Validate even an already committed retry; an ID cannot acquire new meaning.
      baseline.preferences.applyProfile(selected);
      final keys = baseline.preferences.profileSettings().keys.toSet();
      if (baseline.revisions.keys.toSet().difference(keys).isNotEmpty ||
          keys.difference(baseline.revisions.keys.toSet()).isNotEmpty ||
          baseline.revisions.values.any(
            (v) => v < 0 || v > _Ledger.maximumRevision,
          )) {
        throw const FormatException(
          'Invalid settings review. Reopen the profile.',
        );
      }
      Map<String, Object?> ordered(Map<String, Object?> value) => {
        for (final key in value.keys.toList()..sort()) key: value[key],
      };
      final request = jsonEncode({
        'values': ordered(baseline.preferences.profileSettings()),
        'revisions': ordered(baseline.revisions),
        'changes': ordered(selected),
      });
      final current = await _read();
      if (current.receipt?['id'] == id) {
        if (current.receipt?['request'] != request) {
          throw const FormatException(
            'This settings review changed. Reopen the profile.',
          );
        }
        return current.result();
      }
      final applied = <String, Object?>{}, kept = <String>[];
      for (final entry in selected.entries) {
        if (current.revisions[entry.key] == baseline.revisions[entry.key] &&
            current.preferences.profileSettings()[entry.key] ==
                baseline.preferences.profileSettings()[entry.key]) {
          applied[entry.key] = entry.value;
        } else {
          kept.add(entry.key);
        }
      }
      final next = current.update(current.preferences.applyProfile(applied));
      next.receipt = {
        'id': id,
        'request': request,
        'applied': applied.keys.toList(),
        'kept': kept,
        'revisions': Map<String, int>.of(next.revisions),
      };
      try {
        await _storage.write(next.encode());
      } catch (_) {
        // A platform write may commit before its reply is lost. Never replay that
        // import over newer edits just because the native receipt is still pending.
        final observed = await _read();
        if (observed.receipt?['id'] != id ||
            observed.receipt?['request'] != request) {
          rethrow;
        }
        return observed.result();
      }
      return next.result();
    });
  }
}

class _Ledger {
  _Ledger(
    this.preferences,
    this.revisions,
    this.clock,
    this.extra,
    this.receipt,
  );
  static const maximumRevision = 9007199254740991;
  final Preferences preferences;
  final Map<String, int> revisions;
  final int clock;
  final Map<String, dynamic> extra;
  Map<String, dynamic>? receipt;
  static _Ledger decode(String? raw) {
    final preferences = Preferences.decode(raw);
    final data = raw == null
        ? <String, dynamic>{}
        : jsonDecode(raw) as Map<String, dynamic>;
    final metadata = data['_profile_preferences'];
    var clock = 0;
    final revisions = {
      for (final key in preferences.profileSettings().keys) key: 0,
    };
    Map<String, dynamic>? receipt;
    if (metadata != null) {
      if (metadata is! Map ||
          metadata['version'] != 1 ||
          metadata['clock'] is! int ||
          metadata['revisions'] is! Map) {
        throw const FormatException(
          'Unknown device preference metadata. Update Shep.',
        );
      }
      clock = metadata['clock'] as int;
      if (clock < 0 || clock > maximumRevision) {
        throw const FormatException('Invalid device preference revision.');
      }
      for (final entry in (metadata['revisions'] as Map).entries) {
        if (!revisions.containsKey(entry.key) ||
            entry.value is! int ||
            entry.value < 0 ||
            entry.value > clock) {
          throw const FormatException('Invalid device preference metadata.');
        }
        revisions[entry.key as String] = entry.value as int;
      }
      if (metadata['receipt'] case final Map value) {
        receipt = Map<String, dynamic>.from(value);
        if (receipt['id'] is! String ||
            receipt['request'] is! String ||
            receipt['applied'] is! List ||
            receipt['kept'] is! List) {
          throw const FormatException(
            'Invalid saved profile preference receipt.',
          );
        }
        final originalRevisions = receipt['revisions'];
        if (originalRevisions != null) {
          if (originalRevisions is! Map ||
              originalRevisions.length != revisions.length ||
              originalRevisions.entries.any(
                (entry) =>
                    !revisions.containsKey(entry.key) ||
                    entry.value is! int ||
                    entry.value < 0 ||
                    entry.value > revisions[entry.key]!,
              )) {
            throw const FormatException(
              'Invalid original preference receipt revisions.',
            );
          }
        }
        final seen = <String>{};
        for (final key in [
          ...receipt['applied'] as List,
          ...receipt['kept'] as List,
        ]) {
          if (key is! String || !revisions.containsKey(key) || !seen.add(key)) {
            throw const FormatException('Invalid saved preference field.');
          }
        }
      } else if (metadata['receipt'] != null) {
        throw const FormatException('Invalid saved preference receipt.');
      }
    }
    return _Ledger(preferences, revisions, clock, data, receipt);
  }

  _Ledger update(Preferences value, {Set<String> intent = const {}}) {
    if (clock == maximumRevision) {
      throw const FormatException('Device preference revision is exhausted.');
    }
    final before = preferences.profileSettings(),
        after = value.profileSettings();
    return _Ledger(
      value,
      {
        for (final key in before.keys)
          key: before[key] == after[key] && !intent.contains(key)
              ? revisions[key]!
              : clock + 1,
      },
      clock + 1,
      extra,
      receipt,
    );
  }

  String encode() => jsonEncode({
    ...extra,
    ...jsonDecode(preferences.encode()) as Map<String, dynamic>,
    '_profile_preferences': {
      'version': 1,
      'clock': clock,
      'revisions': revisions,
      'receipt': receipt,
    },
  });
  ProfileSettingsReceipt result() => ProfileSettingsReceipt(
    id: receipt!['id'] as String,
    preferences: preferences,
    applied: (receipt!['applied'] as List).cast<String>(),
    kept: (receipt!['kept'] as List).cast<String>(),
    revisions: Map<String, int>.unmodifiable(
      (receipt!['revisions'] as Map?)?.cast<String, int>() ?? const {},
    ),
  );
}
