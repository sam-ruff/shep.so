import 'package:flutter/material.dart';
import '../model/formatted_message.dart';
import '../model/remote_images.dart';
import '../model/workspace.dart';
import 'controls.dart';
import 'icons.dart';
import 'theme.dart';

/// The synced policy, this device's exceptions and the Contacts list.
class ImagePreferences extends StatefulWidget {
  const ImagePreferences({super.key, required this.workspace});
  final Workspace workspace;
  @override
  State<ImagePreferences> createState() => _ImagePreferencesState();
}

class _ImagePreferencesState extends State<ImagePreferences> {
  final contacts = TextEditingController();
  String? contactError;
  Workspace get workspace => widget.workspace;

  @override
  void dispose() {
    contacts.dispose();
    super.dispose();
  }

  Future<void> addContacts() async {
    final error = await workspace.addContacts(contacts.text);
    if (!mounted) return;
    setState(() => contactError = error);
    if (error == null) contacts.clear();
  }

  Widget chips(List<String> values, void Function(String) remove) => Padding(
    padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
    child: Wrap(
      spacing: 6,
      runSpacing: 6,
      children: [
        for (final value in values)
          InputChip(
            key: ValueKey('image-exception-$value'),
            label: Text(value),
            deleteIcon: const ShepIcon('close', size: 14),
            deleteButtonTooltipMessage: 'Remove $value',
            onDeleted: () => remove(value),
          ),
      ],
    ),
  );

  @override
  Widget build(BuildContext context) {
    final p = workspace.preferences;
    final trust = p.imageTrust;
    final c = ShepColors.of(context);
    String count(int n, String noun) => '$n $noun${n == 1 ? '' : 's'}';
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ListTile(
          key: const ValueKey('preference-images'),
          leading: const ShepIcon('shield'),
          title: const Text('External images'),
          subtitle: const Text(
            'Loading an external image lets its server see the request. Sender identity in email is not verified.',
          ),
          trailing: pickList<ImagePolicy>(
            context,
            value: p.imagePolicy,
            onChanged: (value) {
              if (value != null) workspace.setImagePolicy(value);
            },
            items: [
              for (final policy in ImagePolicy.values)
                DropdownMenuItem(value: policy, child: Text(policy.label)),
            ],
          ),
        ),
        ListTile(
          key: const ValueKey('preference-image-exceptions'),
          title: const Text('Image exceptions on this device'),
          subtitle: Text(
            '${count(trust.messages.length, 'message')}, ${count(trust.senders.length, 'sender')}, ${count(trust.domains.length, 'domain')}',
          ),
          trailing: TextButton(
            key: const ValueKey('preference-image-exceptions-clear'),
            onPressed: trust.hasExceptions
                ? workspace.clearImageExceptions
                : null,
            child: const Text('Clear'),
          ),
        ),
        if (trust.senders.isNotEmpty)
          chips(trust.senders, workspace.removeImageSender),
        if (trust.domains.isNotEmpty)
          chips(trust.domains, workspace.removeImageDomain),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 8),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(
                child: TextField(
                  key: const ValueKey('preference-contacts'),
                  controller: contacts,
                  keyboardType: TextInputType.emailAddress,
                  autocorrect: false,
                  decoration: InputDecoration(
                    labelText: 'Add contacts',
                    hintText: 'alex@example.com, maya@example.com',
                    helperText: 'The Contacts only policy loads their images.',
                    helperMaxLines: 2,
                    errorText: contactError,
                    errorMaxLines: 3,
                  ),
                  onSubmitted: (_) => addContacts(),
                ),
              ),
              const SizedBox(width: 8),
              Padding(
                padding: const EdgeInsets.only(top: 6),
                child: OutlinedButton(
                  key: const ValueKey('preference-contacts-add'),
                  style: const ButtonStyle(
                    minimumSize: WidgetStatePropertyAll(Size(0, 44)),
                  ),
                  onPressed: addContacts,
                  child: const Text('Add'),
                ),
              ),
            ],
          ),
        ),
        if (trust.contacts.isNotEmpty)
          chips(trust.contacts, workspace.removeContact)
        else
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
            child: Text(
              'No contacts saved.',
              style: TextStyle(color: c.muted, fontSize: ShepText.secondary),
            ),
          ),
      ],
    );
  }
}

/// External image status and choices for one formatted message.
class RemoteImageBar extends StatelessWidget {
  const RemoteImageBar({
    super.key,
    required this.workspace,
    required this.document,
    required this.id,
  });
  final Workspace workspace;
  final FormattedMessage document;
  final String id;

  @override
  Widget build(BuildContext context) {
    final prepared = document.prepared!;
    final c = ShepColors.of(context);
    final count = prepared.remoteImages.length;
    final noun = count == 1 ? 'remote image' : 'remote images';
    final address = prepared.senderAddress, domain = prepared.senderDomain;
    final grant = workspace.preferences.imageRules.grant(
      id,
      address: address,
      domain: domain,
    );
    final muted = TextStyle(color: c.muted, fontSize: ShepText.secondary);
    final style = ButtonStyle(
      visualDensity: VisualDensity.standard,
      minimumSize: const WidgetStatePropertyAll(Size(0, 44)),
    );
    Widget status(String text, {bool error = false}) => Semantics(
      liveRegion: true,
      child: Text(
        text,
        key: const ValueKey('images-status'),
        style: error ? muted.copyWith(color: c.flag) : muted,
      ),
    );
    final children = <Widget>[ShepIcon('image', size: 18, color: c.muted)];
    if (grant == null || !document.supportsImages) {
      children.add(status('$count $noun blocked.'));
      if (document.supportsImages) {
        children.add(
          FilledButton.tonal(
            key: const ValueKey('images-load'),
            style: style,
            onPressed: () => workspace.allowImages(ImageGrant.message, id: id),
            child: const Text('Load images'),
          ),
        );
        if (address != null) {
          children.add(
            PopupMenuButton<ImageGrant>(
              key: const ValueKey('images-more'),
              tooltip: 'More image choices',
              icon: const ShepIcon('more'),
              onSelected: (scope) => workspace.allowImages(
                scope,
                id: id,
                address: address,
                domain: domain,
              ),
              itemBuilder: (_) => [
                PopupMenuItem(
                  value: ImageGrant.sender,
                  child: Text('Always for $address'),
                ),
                if (domain != null)
                  PopupMenuItem(
                    value: ImageGrant.domain,
                    child: Text('Always for $domain'),
                  ),
              ],
            ),
          );
        }
      }
    } else {
      final failed = document.imageErrors.length;
      final limited = count > FormattedMessage.maxRemoteImages;
      if (document.imagesLoading) {
        children.add(status('Loading $count $noun'));
        children.add(
          const SizedBox(
            width: 16,
            height: 16,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
        );
      } else if (document.imagesError case final String message) {
        children.add(status(message, error: true));
      } else if (failed > 0) {
        children.add(
          status('$failed of $count $noun could not load.', error: true),
        );
      } else {
        children.add(
          status(
            limited
                ? 'Showing the first ${FormattedMessage.maxRemoteImages} of $count $noun.'
                : '$count $noun allowed.',
          ),
        );
      }
      if (!document.imagesLoading &&
          (failed > 0 || document.imagesError != null)) {
        children.add(
          TextButton(
            key: const ValueKey('images-retry'),
            style: style,
            onPressed: () => document.loadImages(
              workspace.preferences.imageRules,
              retry: true,
            ),
            child: const Text('Retry images'),
          ),
        );
      }
      if (grant == ImageGrant.policy || grant == ImageGrant.contact) {
        children.add(
          Text(
            grant == ImageGrant.policy
                ? 'Allowed for all senders in Preferences.'
                : 'Allowed for your contacts in Preferences.',
            style: muted,
          ),
        );
      } else {
        children.add(
          TextButton(
            key: const ValueKey('images-block'),
            style: style,
            onPressed: () =>
                workspace.blockImages(id: id, address: address, domain: domain),
            child: const Text('Block images'),
          ),
        );
      }
    }
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Wrap(
        spacing: 8,
        runSpacing: 4,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: children,
      ),
    );
  }
}
