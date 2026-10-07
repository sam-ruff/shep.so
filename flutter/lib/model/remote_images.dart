import 'package:flutter/foundation.dart';

/// The synced default for external images, matching desktop's wire values.
enum ImagePolicy {
  blockAll('BlockAll', 'Block all'),
  contacts('Contacts', 'Contacts only'),
  allowAll('AllowAll', 'Allow all');

  const ImagePolicy(this.wire, this.label);
  final String wire, label;
  static ImagePolicy? parse(Object? value) =>
      values.where((policy) => policy.wire == value).firstOrNull;
}

/// How a message's images are allowed, so the reader can offer to revoke it.
enum ImageGrant { policy, contact, message, sender, domain }

final _address = RegExp(r'^[^\s@<>,;"()]+@[^\s@<>,;"()]+\.[^\s@<>,;"()]+$');

/// Lower-cased addresses from comma, semicolon or space separated text, or
/// null when any entry is not a plain address.
List<String>? parseContacts(String text) {
  final entries = text
      .split(RegExp(r'[\s,;]+'))
      .where((entry) => entry.isNotEmpty)
      .toList();
  if (entries.isEmpty ||
      entries.any((entry) => entry.length > 320 || !_address.hasMatch(entry))) {
    return null;
  }
  return entries.map((entry) => entry.toLowerCase()).toSet().toList();
}

/// Explicit exceptions kept on this device. Message IDs are local cache
/// identities, so none of these lists is published to the shared profile.
@immutable
class ImageTrust {
  const ImageTrust({
    this.messages = const [],
    this.senders = const [],
    this.domains = const [],
    this.contacts = const [],
  });
  final List<String> messages, senders, domains, contacts;

  /// Oldest entries drop first so stored preferences stay bounded.
  static const limit = 500;

  bool get hasExceptions =>
      messages.isNotEmpty || senders.isNotEmpty || domains.isNotEmpty;

  static List<String> _add(List<String> list, String value) => [
    ...list.where((item) => item != value),
    value,
  ].reversed.take(limit).toList().reversed.toList();

  ImageTrust allowMessage(String id) => _copy(messages: _add(messages, id));
  ImageTrust allowSender(String address) =>
      _copy(senders: _add(senders, address.toLowerCase()));
  ImageTrust allowDomain(String domain) =>
      _copy(domains: _add(domains, domain.toLowerCase()));
  ImageTrust addContact(String address) =>
      _copy(contacts: _add(contacts, address.toLowerCase()));

  ImageTrust revoke({String? message, String? sender, String? domain}) => _copy(
    messages: messages.where((value) => value != message).toList(),
    senders: senders.where((value) => value != sender).toList(),
    domains: domains.where((value) => value != domain).toList(),
  );
  ImageTrust removeContact(String address) => _copy(
    contacts: contacts
        .where((value) => value.toLowerCase() != address.toLowerCase())
        .toList(),
  );
  ImageTrust clearExceptions() => ImageTrust(contacts: contacts);

  ImageTrust _copy({
    List<String>? messages,
    List<String>? senders,
    List<String>? domains,
    List<String>? contacts,
  }) => ImageTrust(
    messages: messages ?? this.messages,
    senders: senders ?? this.senders,
    domains: domains ?? this.domains,
    contacts: contacts ?? this.contacts,
  );

  Map<String, Object?> toJson() => {
    'messages': messages,
    'senders': senders,
    'domains': domains,
    'contacts': contacts,
  };

  /// Tolerates absent or damaged storage: a bad list grants nothing.
  static ImageTrust fromJson(Object? value) {
    if (value is! Map) return const ImageTrust();
    List<String> list(String key) {
      final items = value[key];
      if (items is! List) return const [];
      return items
          .whereType<String>()
          .where((item) => item.isNotEmpty && item.length <= 512)
          .take(limit)
          .toList();
    }

    return ImageTrust(
      messages: list('messages'),
      senders: list('senders'),
      domains: list('domains'),
      contacts: list('contacts'),
    );
  }

  @override
  bool operator ==(Object other) =>
      other is ImageTrust &&
      listEquals(other.messages, messages) &&
      listEquals(other.senders, senders) &&
      listEquals(other.domains, domains) &&
      listEquals(other.contacts, contacts);
  @override
  int get hashCode => Object.hash(
    Object.hashAll(messages),
    Object.hashAll(senders),
    Object.hashAll(domains),
    Object.hashAll(contacts),
  );
}

/// The same decision as the native check in `shared/mail-core`; the native
/// service still enforces it before any request.
@immutable
class ImageRules {
  const ImageRules(this.policy, this.trust);
  final ImagePolicy policy;
  final ImageTrust trust;

  /// `address` and `domain` are the native parse of the From header. Policy
  /// grants come first because removing an exception cannot revoke them.
  ImageGrant? grant(String id, {String? address, String? domain}) {
    if (policy == ImagePolicy.allowAll) return ImageGrant.policy;
    if (policy == ImagePolicy.contacts &&
        address != null &&
        trust.contacts.any(
          (contact) => contact.toLowerCase() == address.toLowerCase(),
        )) {
      return ImageGrant.contact;
    }
    if (trust.messages.contains(id)) return ImageGrant.message;
    if (address != null && trust.senders.contains(address)) {
      return ImageGrant.sender;
    }
    if (domain != null && trust.domains.contains(domain)) {
      return ImageGrant.domain;
    }
    return null;
  }

  bool allows(String id, {String? address, String? domain}) =>
      grant(id, address: address, domain: domain) != null;

  /// Whether `next` could hide an image these rules allowed.
  bool narrowedBy(ImageRules next) {
    bool lost(List<String> before, List<String> after) =>
        before.any((value) => !after.contains(value));
    final rank = [
      ImagePolicy.blockAll,
      ImagePolicy.contacts,
      ImagePolicy.allowAll,
    ];
    return rank.indexOf(next.policy) < rank.indexOf(policy) ||
        lost(trust.messages, next.trust.messages) ||
        lost(trust.senders, next.trust.senders) ||
        lost(trust.domains, next.trust.domains) ||
        (policy == ImagePolicy.contacts &&
            lost(trust.contacts, next.trust.contacts));
  }

  Map<String, Object?> toJson() => {'policy': policy.wire, ...trust.toJson()};
}
