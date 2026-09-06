import 'package:shared_preferences/shared_preferences.dart';
import '../model/preferences.dart';

abstract interface class SettingsStore {
  Future<Preferences> read();
  Future<void> write(Preferences value);
}

/// Non-secret appearance/interaction preferences only; never mail or credentials.
class DeviceSettings implements SettingsStore {
  final _store = SharedPreferencesAsync();
  @override
  Future<Preferences> read() async =>
      Preferences.decode(await _store.getString('shep.preferences.v1'));
  @override
  Future<void> write(Preferences value) =>
      _store.setString('shep.preferences.v1', value.encode());
}
