part of 'workspace.dart';

/// External image choices. Narrowing a permission also drops the native cache
/// so a revoked image cannot reappear in a reader or a print.
extension RemoteImagePreferences on Workspace {
  Future<void> setImagePolicy(ImagePolicy policy) =>
      _saveImageRules(preferences.copy(imagePolicy: policy));

  Future<void> allowImages(
    ImageGrant scope, {
    required String id,
    String? address,
    String? domain,
  }) {
    final trust = preferences.imageTrust;
    final next = switch (scope) {
      ImageGrant.message => trust.allowMessage(id),
      ImageGrant.sender when address != null => trust.allowSender(address),
      ImageGrant.domain when domain != null => trust.allowDomain(domain),
      _ => trust,
    };
    return _saveImageRules(preferences.copy(imageTrust: next));
  }

  /// Removes every exception that allows this message.
  Future<void> blockImages({
    required String id,
    String? address,
    String? domain,
  }) => _saveImageRules(
    preferences.copy(
      imageTrust: preferences.imageTrust.revoke(
        message: id,
        sender: address,
        domain: domain,
      ),
    ),
  );

  Future<void> removeImageSender(String address) => _saveImageRules(
    preferences.copy(
      imageTrust: preferences.imageTrust.revoke(sender: address),
    ),
  );

  Future<void> removeImageDomain(String domain) => _saveImageRules(
    preferences.copy(imageTrust: preferences.imageTrust.revoke(domain: domain)),
  );

  Future<void> clearImageExceptions() => _saveImageRules(
    preferences.copy(imageTrust: preferences.imageTrust.clearExceptions()),
  );

  /// Returns an explanation instead of saving when any address is invalid.
  Future<String?> addContacts(String text) async {
    final addresses = parseContacts(text);
    if (addresses == null) {
      return 'Enter email addresses separated by commas, such as alex@example.com.';
    }
    var trust = preferences.imageTrust;
    for (final address in addresses) {
      trust = trust.addContact(address);
    }
    await _saveImageRules(preferences.copy(imageTrust: trust));
    return null;
  }

  Future<void> removeContact(String address) => _saveImageRules(
    preferences.copy(imageTrust: preferences.imageTrust.removeContact(address)),
  );

  Future<void> _saveImageRules(Preferences next) async {
    final narrowed = preferences.imageRules.narrowedBy(next.imageRules);
    final saved = savePreferences(next);
    final images = repository;
    if (narrowed && images is RemoteImageRepository) {
      try {
        await (images as RemoteImageRepository).forgetRemoteImages();
      } catch (_) {
        error =
            'Could not clear cached images. Close and reopen Shep to clear them.';
        _changed();
      }
    }
    await saved;
  }
}
