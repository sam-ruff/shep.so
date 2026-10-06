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
    for (final (key, expected) in [
      ('subject', headerSubject),
      ('sender', headerSender),
      ('address', 'sender@example.test'),
      ('recipient', headerRecipient),
    ]) {
      final control = find.byKey(ValueKey('reader-$key'));
      expect(tester.widget<SelectableText>(control).data, expected);
      final copy = find.byKey(ValueKey('reader-copy-$key'));
      await tester.ensureVisible(copy);
      await tester.tap(copy);
      await tester.pump();
      if (clipboard != null) expect(await clipboard(), expected);
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
    final copy = find.byKey(const ValueKey('reader-copy-recipient'));
    await tester.ensureVisible(copy);
    await tester.tap(copy);
    await tester.pump();
    if (clipboard != null) {
      expect(await clipboard(), 'Alias target <target@example.test>');
    }
    expect(tester.element(footer), same(footerElement));
    expect(tester.takeException(), isNull);
  } finally {
    await tester.pumpWidget(const SizedBox.shrink());
    workspace.dispose();
  }
}
