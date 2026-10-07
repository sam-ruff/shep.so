import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/formatted_message.dart';
import 'package:shep_mobile/data/profile_settings.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/formatted_message.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/remote_images.dart';
import 'profile_settings_test.dart' show Bytes;

String imageKey(int index) => index.toRadixString(16).padLeft(64, '0');

class Images implements FormattedMessageRepository, RemoteImageRepository {
  Images(this.count);
  final int count;
  final requests =
      <
        ({
          List<String> keys,
          ImageRules rules,
          Completer<RemoteImageBatch> reply,
        })
      >[];
  int forgotten = 0;
  @override
  Future<PreparedMessage> formattedMessage(
    String id, {
    required String generation,
    required bool dark,
    required bool quotes,
  }) async => PreparedMessage(
    signature: id,
    text: 'Plain',
    document: 'confined fixture',
    senderAddress: 'news@example.test',
    senderDomain: 'example.test',
    remoteImages: [
      for (var i = 0; i < count; i++)
        RemoteImage(
          'https://images.example.test/$i.png',
          'Image $i',
          key: imageKey(i),
        ),
    ],
  );
  @override
  Future<RemoteImageBatch> remoteImages(
    String id, {
    required List<String> keys,
    required ImageRules rules,
  }) {
    final reply = Completer<RemoteImageBatch>();
    requests.add((keys: keys, rules: rules, reply: reply));
    return reply.future;
  }

  @override
  Future<void> forgetRemoteImages() async => forgotten++;
}

RemoteImageBatch loaded(Iterable<String> keys, {Map<String, String>? failed}) =>
    RemoteImageBatch(
      images: {
        for (final key in keys) key: const RemoteImageBytes('UklG', 1, 1),
      },
      failed: failed ?? const {},
    );

ImageRules allowMessage(String id) =>
    ImageRules(ImagePolicy.blockAll, ImageTrust(messages: [id]));

Future<void> settle() => Future<void>.delayed(Duration.zero);

void main() {
  test('shared policy cases agree with the native decision', () {
    final cases =
        (jsonDecode(
                  File('../shared/remote-image-cases.json').readAsStringSync(),
                )
                as Map)['cases']
            as List;
    expect(cases.length, greaterThanOrEqualTo(10));
    for (final value in cases.cast<Map>()) {
      final rules = (value['rules'] as Map).cast<String, Object?>();
      List<String> list(String key) =>
          (rules[key] as List? ?? const []).cast<String>();
      final decision = ImageRules(
        ImagePolicy.parse(rules['policy'])!,
        ImageTrust(
          messages: list('messages'),
          senders: list('senders'),
          domains: list('domains'),
          contacts: list('contacts'),
        ),
      );
      expect(
        decision.allows(
          value['message'] as String,
          address: value['address'] as String?,
          domain: value['domain'] as String?,
        ),
        value['allowed'],
        reason: value['name'] as String,
      );
    }
  });

  test('exceptions stay bounded and revocable, and narrowing is detected', () {
    var trust = const ImageTrust().allowSender('Ada@Example.test');
    trust = trust.allowSender('ada@example.test').allowDomain('Example.test');
    expect(trust.senders, ['ada@example.test']);
    expect(trust.domains, ['example.test']);
    for (var i = 0; i < ImageTrust.limit + 5; i++) {
      trust = trust.allowMessage('m$i');
    }
    expect(trust.messages, hasLength(ImageTrust.limit));
    expect(trust.messages.first, 'm5');
    expect(trust.messages.last, 'm${ImageTrust.limit + 4}');
    final contacts = trust.addContact('Maya@example.test');
    expect(contacts.clearExceptions(), ImageTrust(contacts: contacts.contacts));
    final revoked = contacts.revoke(
      message: 'm9',
      sender: 'ada@example.test',
      domain: 'example.test',
    );
    expect(revoked.messages, isNot(contains('m9')));
    expect(revoked.senders, isEmpty);
    expect(revoked.domains, isEmpty);
    expect(revoked.contacts, ['maya@example.test']);

    final base = ImageRules(ImagePolicy.contacts, contacts);
    expect(base.narrowedBy(ImageRules(ImagePolicy.allowAll, contacts)), false);
    expect(base.narrowedBy(ImageRules(ImagePolicy.blockAll, contacts)), true);
    expect(base.narrowedBy(ImageRules(ImagePolicy.contacts, revoked)), true);
    expect(
      base.narrowedBy(
        ImageRules(ImagePolicy.contacts, contacts.allowSender('x@y.test')),
      ),
      false,
    );
    expect(
      ImageRules(ImagePolicy.blockAll, contacts).narrowedBy(
        ImageRules(
          ImagePolicy.blockAll,
          contacts.removeContact('maya@example.test'),
        ),
      ),
      false,
    );
    expect(
      base.narrowedBy(
        ImageRules(
          ImagePolicy.contacts,
          contacts.removeContact('MAYA@example.test'),
        ),
      ),
      true,
    );
  });

  test('contacts accept only plain addresses', () {
    expect(parseContacts('Alex@Example.com, maya@example.com;\nkim@b.org'), [
      'alex@example.com',
      'maya@example.com',
      'kim@b.org',
    ]);
    for (final invalid in [
      '',
      'Alex <alex@example.com>',
      'alex',
      'alex@localhost',
      'alex@example.com, bad',
    ]) {
      expect(parseContacts(invalid), isNull, reason: invalid);
    }
  });

  test('the policy syncs as a profile setting while exceptions stay local', () {
    final trust = const ImageTrust().allowSender('ada@example.test');
    final preferences = Preferences(
      imagePolicy: ImagePolicy.contacts,
      imageTrust: trust,
    );
    final settings = preferences.profileSettings();
    expect(settings['image_policy'], 'Contacts');
    expect(settings.keys.where((key) => key.contains('image')).toList(), [
      'image_policy',
    ]);
    final restored = Preferences.decode(preferences.encode());
    expect(restored.imagePolicy, ImagePolicy.contacts);
    expect(restored.imageTrust, trust);
    final remote = preferences.applyProfile({'image_policy': 'AllowAll'});
    expect(remote.imagePolicy, ImagePolicy.allowAll);
    expect(remote.imageTrust, trust);
    expect(
      preferences.applyProfile({'image_policy': null}).imagePolicy,
      ImagePolicy.blockAll,
    );
    expect(
      () => preferences.applyProfile({'image_policy': 'Everything'}),
      throwsFormatException,
    );
    final old = Preferences.decode(jsonEncode({'version': 1}));
    expect(old.imagePolicy, ImagePolicy.blockAll);
    expect(old.imageTrust, const ImageTrust());
    final damaged = Preferences.decode(
      jsonEncode({
        'version': 1,
        'imagePolicy': 'Sometimes',
        'imageTrust': {
          'senders': [7, '', 'ada@example.test'],
          'domains': 'example.test',
        },
      }),
    );
    expect(damaged.imagePolicy, ImagePolicy.blockAll);
    expect(damaged.imageTrust.senders, ['ada@example.test']);
    expect(damaged.imageTrust.domains, isEmpty);
  });

  test(
    'device storage keeps exceptions with local saves and accepts older field sets',
    () async {
      final bytes = Bytes();
      final store = DeviceSettings(storage: bytes);
      final trust = const ImageTrust().allowDomain('example.test');
      final saved = await store.saveLocal({
        'image_policy': 'AllowAll',
      }, imageTrust: trust);
      expect(saved.imagePolicy, ImagePolicy.allowAll);
      final reopened = DeviceSettings(storage: bytes);
      expect((await reopened.read()).imageTrust, trust);
      final snapshot = await reopened.profileSnapshot();
      expect(snapshot.revisions['image_policy'], greaterThan(0));
      final receipt = await reopened.applyProfile(
        id: 'remote-images',
        baseline: snapshot,
        changes: {'image_policy': 'BlockAll'},
      );
      expect(receipt.applied, ['image_policy']);
      expect(receipt.preferences.imageTrust, trust);
      // A save that names no exceptions leaves the saved ones alone.
      await reopened.saveLocal({'tooltips': false});
      expect((await reopened.read()).imageTrust, trust);

      final current = const Preferences().profileSettings().keys.toSet();
      expect(validProfileFieldSet(current), isTrue);
      expect(
        validProfileFieldSet({...current}..remove('image_policy')),
        isTrue,
      );
      expect(
        validProfileFieldSet(
          {...current}
            ..remove('image_policy')
            ..remove('reply_include_original'),
        ),
        isTrue,
      );
      expect(
        validProfileFieldSet({...current}..remove('reply_include_original')),
        isFalse,
      );
      expect(validProfileFieldSet({...current, 'image_senders'}), isFalse);
      final nine = ProfileSettingsSnapshot(const Preferences(), {
        for (final key in current)
          if (key != 'image_policy') key: 0,
      });
      expect(nine.values.containsKey('image_policy'), isFalse);
      expect(nine.values, hasLength(9));
    },
  );

  test(
    'permitted images reach the document in batches after ready and late results are dropped',
    () async {
      final repository = Images(10);
      final model = FormattedMessage(repository, 'message');
      await model.load(dark: false, quotes: false);
      final commands = <Map<String, Object?>>[];
      final subscription = model.commands.stream.listen(commands.add);
      expect(model.supportsImages, isTrue);
      expect(model.imagesPending, isTrue);
      final loading = model.loadImages(allowMessage('message'));
      expect(model.imagesLoading, isTrue);
      expect(repository.requests.single.keys, [
        for (var i = 0; i < 8; i++) imageKey(i),
      ]);
      repository.requests.single.reply.complete(
        loaded([for (var i = 0; i < 8; i++) imageKey(i)]),
      );
      await settle();
      expect(commands, isEmpty, reason: 'Delivery waits for the runtime');
      expect(repository.requests.last.keys, [imageKey(8), imageKey(9)]);
      repository.requests.last.reply.complete(
        loaded(
          [imageKey(8)],
          failed: {imageKey(9): 'The image server did not return this image.'},
        ),
      );
      await loading;
      expect(model.imagesLoading, isFalse);
      expect(model.imagesPending, isFalse);
      expect(model.imageErrors.keys, [imageKey(9)]);
      model.receive({'type': 'ready', 'generation': model.generation});
      final delivered = commands.where((c) => c['type'] == 'images').toList();
      expect(delivered, hasLength(2));
      expect(delivered.first['generation'], model.generation);
      expect((delivered.first['images'] as Map).keys, hasLength(8));
      expect(
        model.receive({
          'type': 'images',
          'generation': model.generation,
          'keys': [imageKey(9)],
        }),
        isFalse,
      );
      model.receive({
        'type': 'images',
        'generation': model.generation,
        'keys': [imageKey(0), imageKey(8)],
      });
      expect(model.shownImages, {imageKey(0), imageKey(8)});

      final retry = model.loadImages(allowMessage('message'), retry: true);
      expect(repository.requests.last.keys, [imageKey(9)]);
      repository.requests.last.reply.complete(loaded([imageKey(9)]));
      await retry;
      expect(model.imageErrors, isEmpty);
      expect(commands.where((c) => c['type'] == 'images'), hasLength(3));
      await subscription.cancel();
      model.dispose();
    },
  );

  test('cancellation, reload and refusals discard late image results', () async {
    final repository = Images(2);
    final model = FormattedMessage(repository, 'message');
    await model.load(dark: false, quotes: false);
    model.receive({'type': 'ready', 'generation': model.generation});
    final commands = <Map<String, Object?>>[];
    final subscription = model.commands.stream.listen(commands.add);

    final cancelled = model.loadImages(allowMessage('message'));
    model.cancelImages();
    expect(model.imagesLoading, isFalse);
    repository.requests.last.reply.complete(loaded([imageKey(0), imageKey(1)]));
    await cancelled;
    expect(model.loadedImages, isEmpty);
    expect(commands.where((c) => c['type'] == 'images'), isEmpty);

    final reloaded = model.loadImages(allowMessage('message'));
    final old = model.generation;
    await model.load(dark: false, quotes: false);
    expect(model.generation, isNot(old));
    repository.requests.last.reply.complete(loaded([imageKey(0)]));
    await reloaded;
    expect(model.loadedImages, isEmpty);
    expect(commands.where((c) => c['type'] == 'images'), isEmpty);

    final refused = model.loadImages(allowMessage('message'));
    repository.requests.last.reply.completeError(
      const MailOperationFailure(
        'Remote images are blocked for this message. Choose Load images to allow them.',
      ),
    );
    await refused;
    expect(model.imagesError, contains('blocked'));
    expect(model.imagesLoading, isFalse);
    final retried = model.loadImages(allowMessage('message'), retry: true);
    expect(model.imagesError, isNull);
    repository.requests.last.reply.complete(loaded([imageKey(0), imageKey(1)]));
    await retried;
    expect(model.loadedImages, {imageKey(0), imageKey(1)});
    await subscription.cancel();
    model.dispose();
  });
}
