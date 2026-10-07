import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:url_launcher/url_launcher.dart';
import '../model/mail.dart';
import '../model/workspace.dart';

/// Reviews a link from a received message before acting on it. Web links open
/// in another app; email links become a new Shep draft holding only their
/// address. Returns that unsent draft when the person chose to write one.
Future<Draft?> reviewMessageLink(
  BuildContext context,
  Workspace workspace,
  String link,
) async {
  final url = Uri.tryParse(link);
  if (url == null ||
      !['https', 'http', 'mailto'].contains(url.scheme) ||
      url.userInfo.isNotEmpty) {
    return null;
  }
  final email = url.scheme == 'mailto';
  String? status;
  var writing = false;
  return showDialog<Draft>(
    context: context,
    builder: (context) => StatefulBuilder(
      builder: (context, update) => AlertDialog(
        title: const Text('Message link'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SelectableText(url.toString()),
            if (status != null)
              Semantics(liveRegion: true, child: Text(status!)),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Close'),
          ),
          TextButton(
            onPressed: () async {
              try {
                await Clipboard.setData(ClipboardData(text: url.toString()));
                if (context.mounted) {
                  update(() => status = 'Address copied.');
                }
              } catch (_) {
                if (context.mounted) {
                  update(() => status = 'Select and copy the address above.');
                }
              }
            },
            child: const Text('Copy address'),
          ),
          if (email)
            FilledButton(
              onPressed: writing
                  ? null
                  : () async {
                      update(() {
                        writing = true;
                        status = null;
                      });
                      try {
                        final draft = await workspace.openMailto(
                          link,
                          message: true,
                        );
                        if (context.mounted) Navigator.pop(context, draft);
                      } catch (e) {
                        if (context.mounted) {
                          update(() {
                            writing = false;
                            status = '$e';
                          });
                        }
                      }
                    },
              child: const Text('Write message'),
            )
          else
            FilledButton(
              onPressed: () async {
                try {
                  final opened = await launchUrl(
                    url,
                    mode: LaunchMode.externalApplication,
                  );
                  if (context.mounted) {
                    if (opened) {
                      Navigator.pop(context);
                    } else {
                      update(
                        () => status =
                            'No application could open this link. Copy the address instead.',
                      );
                    }
                  }
                } catch (_) {
                  if (context.mounted) {
                    update(
                      () => status =
                          'Could not open this link. Copy the address instead.',
                    );
                  }
                }
              },
              child: const Text('Open link'),
            ),
        ],
      ),
    ),
  );
}
