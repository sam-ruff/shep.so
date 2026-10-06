import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/reader.dart';
import 'package:shep_mobile/ui/theme.dart';
import '../workspace_test.dart' show MemorySettings;
import 'reader_headers_repository.dart';
export 'reader_headers_repository.dart';

Future<void> readerHeadersScenario(
  WidgetTester tester, {
  required bool dark,
  Future<String?> Function()? clipboard,
  Future<void> Function(String)? capture,
}) async {
  final repository = HeaderRepository();
  final workspace = Workspace(repository, MemorySettings());
  Future<void> copy(String key, String label, String expected) async {
    final control = find.byKey(ValueKey('reader-copy-$key'));
    await tester.ensureVisible(control);
    expect(find.text('$label copied.'), findsNothing);
    await tester.tap(control);
    final deadline = DateTime.now().add(const Duration(seconds: 20));
    while (find.text('$label copied.').evaluate().isEmpty) {
      if (DateTime.now().isAfter(deadline)) {
        fail('Copy $label did not complete.');
      }
      await tester.pump(const Duration(milliseconds: 50));
    }
    if (clipboard != null) expect(await clipboard(), expected);
  }

  await workspace.loadPage();
  try {
    await tester.pumpWidget(
      MaterialApp(
        theme: shepTheme(dark ? Brightness.dark : Brightness.light),
        home: Reader(
          workspace: workspace,
          id: repository.metadata.id,
          act: (_, _) {},
        ),
      ),
    );
    await tester.pump();
    final pending = workspace.loadBody(repository.metadata.id);
    await tester.pump();
    final footer = find.widgetWithText(OutlinedButton, 'Move');
    final footerElement = tester.element(footer);
    final footerBounds = tester.getRect(footer);
    for (final (key, label, expected) in [
      ('subject', 'subject', headerSubject),
      ('sender', 'sender', headerSender),
      ('address', 'sender address', 'sender@example.test'),
      ('recipient', 'To', headerRecipient),
    ]) {
      final control = find.byKey(ValueKey('reader-$key'));
      expect(tester.widget<SelectableText>(control).data, expected);
      await copy(key, label, expected);
    }
    expect(find.text('Account: Receiving account'), findsOneWidget);
    expect(workspace.loadingBody(repository.metadata.id), isTrue);
    await capture?.call('reader-headers-${dark ? 'dark' : 'light'}-pending');
    repository.metadata = headerMail(
      recipient: 'Updated <updated@example.test>',
    );
    await workspace.loadPage();
    await tester.pump();
    expect(
      tester
          .widget<SelectableText>(
            find.byKey(const ValueKey('reader-recipient')),
          )
          .data,
      'Updated <updated@example.test>',
    );
    repository.body.complete(
      headerMail(
        recipient: headerRecipient,
        body: 'Loaded source body',
        loaded: true,
      ),
    );
    await pending;
    await tester.pump();
    expect(
      tester
          .widget<SelectableText>(
            find.byKey(const ValueKey('reader-recipient')),
          )
          .data,
      'Updated <updated@example.test>',
    );
    expect(tester.element(footer), same(footerElement));
    expect(tester.getRect(footer), footerBounds);
    repository.aliases = {'header-message': 'provider-alias'};
    repository.metadata = headerMail(
      id: 'provider-alias',
      recipient: 'Alias target <target@example.test>',
      body: 'Loaded source body',
      loaded: true,
    );
    await workspace.loadPage();
    await tester.pump();
    expect(
      tester
          .widget<SelectableText>(
            find.byKey(const ValueKey('reader-recipient')),
          )
          .data,
      'Alias target <target@example.test>',
    );
    await copy('recipient', 'To', 'Alias target <target@example.test>');
    expect(tester.element(footer), same(footerElement));
    expect(tester.takeException(), isNull);
  } finally {
    await tester.pumpWidget(const SizedBox.shrink());
    workspace.dispose();
  }
}
