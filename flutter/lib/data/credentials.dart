import 'dart:convert';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

abstract interface class CredentialStore {
  Future<String?> read(String account, bool smtp);
  Future<void> save(String account, String incoming, String smtp);
  Future<void> remove(String account);
}

class DeviceCredentials implements CredentialStore {
  const DeviceCredentials();
  static const _store = FlutterSecureStorage(
    iOptions: IOSOptions(
      accessibility: KeychainAccessibility.first_unlock_this_device,
    ),
  );
  static Future<void> _writes = Future.value();
  String _key(String id) => 'so.shep.mail.$id.credentials.v1';

  // A complete credential pair uses one platform write. A partial reconnect
  // cannot mix a new incoming password with an older SMTP password.
  Future<void> _ordered(Future<void> Function() operation) {
    final result = _writes.then((_) => operation());
    _writes = result.then<void>((_) {}, onError: (Object _, StackTrace _) {});
    return result;
  }

  @override
  Future<String?> read(String account, bool smtp) async {
    await _writes;
    final value = await _store.read(key: _key(account));
    if (value == null) return null;
    return (jsonDecode(value) as Map<String, dynamic>)[smtp
            ? 'smtp'
            : 'incoming']
        as String?;
  }

  @override
  Future<void> save(String account, String incoming, String smtp) => _ordered(
    () => _store.write(
      key: _key(account),
      value: jsonEncode({'incoming': incoming, 'smtp': smtp}),
    ),
  );
  @override
  Future<void> remove(String account) =>
      _ordered(() => _store.delete(key: _key(account)));
}
