import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/ui/composer.dart';
import 'package:shep_mobile/ui/message_link.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'composer_lifecycle_test.dart' show loadPreviewFonts;
import 'support/mailto_fixture.dart';
import 'support/preview_repository.dart';
import 'workspace_test.dart' show MemorySettings;

const launchLink =
    'mailto:friend@example.test?cc=copy@example.test&bcc=hidden@example.test&subject=Hello&body=First%0ASecond';
const launchFields = Draft(
  id: '',
  to: 'friend@example.test',
  cc: 'copy@example.test',
  bcc: 'hidden@example.test',
  subject: 'Hello',
  body: 'First\nSecond',
);

String field(WidgetTester tester, String label) => tester
    .widget<TextField>(find.widgetWithText(TextField, label))
    .controller!
    .text;

Future<void> settle(WidgetTester tester) async {
  for (var i = 0; i < 30; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
}

void main() {
  late Workspace workspace;
  late MailtoRepositoryFixture repository;
  late MailtoLinksFixture links;

  Future<void> start(
    WidgetTester tester, {
    bool initialize = true,
    List<MailAccount> accounts = const [mailtoAccount],
  }) async {
    tester.view.physicalSize = const Size(412, 892);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    repository = MailtoRepositoryFixture(accounts: accounts)
      ..fields[launchLink] = launchFields;
    workspace = Workspace(repository, MemorySettings());
    await tester.pumpWidget(ShepApp(workspace: workspace, links: links));
    if (initialize) {
      await workspace.initialize();
      await settle(tester);
    }
  }

  Future<void> finish(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    workspace.dispose();
  }

  setUp(() => links = MailtoLinksFixture());

  testWidgets(
    'a launch link waits for the saved workspace, then opens one unsent draft',
    (tester) async {
      links.pending.add(launchLink);
      await start(tester, initialize: false);
      await settle(tester);
      expect(links.takes, 0, reason: 'accounts and drafts are not loaded yet');
      expect(find.widgetWithText(AppBar, 'New message'), findsNothing);
      await workspace.initialize();
      await settle(tester);
      expect(links.takes, 1);
      expect(find.widgetWithText(AppBar, 'New message'), findsOneWidget);
      expect(field(tester, 'To'), 'friend@example.test');
      expect(field(tester, 'Cc'), 'copy@example.test');
      expect(field(tester, 'Bcc'), 'hidden@example.test');
      expect(field(tester, 'Subject'), 'Hello');
      expect(field(tester, 'Message'), 'First\nSecond');
      final request = repository.requests.single;
      expect(request.message, isFalse);
      expect(request.account, mailtoAccount.id);
      expect(workspace.drafts.keys, [request.id]);
      expect(repository.sent, isEmpty, reason: 'a link never sends mail');
      expect(find.byKey(const ValueKey('composer-no-account')), findsNothing);
      await finish(tester);
    },
  );

  testWidgets(
    'typing in a link draft keeps the keyboard on the same field after the first save',
    (tester) async {
      links.pending.add(launchLink);
      await start(tester);
      final message = find.widgetWithText(TextField, 'Message');
      await tester.tap(message);
      await tester.pump();
      for (final text in ['First\nSecond.', 'First\nSecond. More']) {
        tester.testTextInput.updateEditingValue(
          TextEditingValue(
            text: text,
            selection: TextSelection.collapsed(offset: text.length),
          ),
        );
        await settle(tester);
        expect(field(tester, 'Message'), text);
        expect(
          tester
              .widget<EditableText>(
                find.descendant(
                  of: message,
                  matching: find.byType(EditableText),
                ),
              )
              .focusNode
              .hasFocus,
          isTrue,
        );
      }
      expect(field(tester, 'To'), 'friend@example.test');
      await tester.tap(find.byTooltip('Save and close'));
      await settle(tester);
      expect(workspace.drafts.values.single.body, 'First\nSecond. More');
      await finish(tester);
    },
  );

  testWidgets(
    'a link arriving while Shep runs opens above an edited draft without changing it',
    (tester) async {
      await start(tester);
      await tester.tap(
        find.widgetWithText(FloatingActionButton, 'New message'),
      );
      await settle(tester);
      await tester.enterText(
        find.widgetWithText(TextField, 'Message'),
        'Typed before the link',
      );
      await tester.pump();
      links.arrive(launchLink);
      await settle(tester);
      expect(field(tester, 'To'), 'friend@example.test');
      expect(field(tester, 'Message'), 'First\nSecond');
      await tester.tap(find.byTooltip('Save and close').last);
      await settle(tester);
      expect(field(tester, 'Message'), 'Typed before the link');
      expect(field(tester, 'To'), '');
      final opened = repository.requests.single.id;
      expect(repository.drafts[opened]?.subject, 'Hello');
      expect(
        repository.drafts.values.where(
          (d) => d.body == 'Typed before the link',
        ),
        hasLength(1),
        reason: 'the earlier draft keeps its own identity',
      );
      expect(repository.sent, isEmpty);
      await tester.tap(find.byTooltip('Save and close'));
      await settle(tester);
      await finish(tester);
    },
  );

  testWidgets(
    'New message keeps one draft identity while the keyboard and text change',
    (tester) async {
      await start(tester);
      await tester.tap(
        find.widgetWithText(FloatingActionButton, 'New message'),
      );
      await settle(tester);
      final message = find.widgetWithText(TextField, 'Message');
      for (final (inset, text) in [
        (300.0, 'First'),
        (0.0, 'First words'),
        (280.0, 'First words kept'),
      ]) {
        tester.view.viewInsets = FakeViewPadding(bottom: inset);
        await tester.pump();
        await tester.enterText(message, text);
        await settle(tester);
      }
      tester.view.resetViewInsets();
      await tester.pump();
      final saved = repository.drafts.values.toList();
      expect(saved.map((d) => d.id).toSet(), hasLength(1));
      expect(saved.single.body, 'First words kept');
      await tester.tap(find.byTooltip('Save and close'));
      await settle(tester);
      expect(repository.drafts, hasLength(1));
      await finish(tester);
    },
  );

  testWidgets('repeated links each open a separate draft in order', (
    tester,
  ) async {
    await start(tester);
    repository.fields['mailto:second@example.test'] = const Draft(
      id: '',
      to: 'second@example.test',
    );
    repository.hold = Completer<void>();
    links.arrive(launchLink);
    await tester.pump();
    links.arrive('mailto:second@example.test');
    await tester.pump();
    repository.hold!.complete();
    await settle(tester);
    expect(repository.requests.map((r) => r.link), [
      launchLink,
      'mailto:second@example.test',
    ]);
    expect(repository.requests.map((r) => r.id).toSet(), hasLength(2));
    expect(field(tester, 'To'), 'second@example.test');
    await tester.tap(find.byTooltip('Save and close'));
    await settle(tester);
    expect(field(tester, 'To'), 'friend@example.test');
    await finish(tester);
  });

  testWidgets('without an account the draft is kept and sending asks for one', (
    tester,
  ) async {
    links.pending.add(launchLink);
    await start(tester, accounts: const []);
    expect(repository.requests.single.account, '');
    expect(field(tester, 'To'), 'friend@example.test');
    expect(find.byKey(const ValueKey('composer-no-account')), findsOneWidget);
    expect(
      find.textContaining('Add an account in Preferences'),
      findsOneWidget,
    );
    expect(workspace.drafts, hasLength(1));
    expect(repository.sent, isEmpty);
    await finish(tester);
  });

  for (final dark in [false, true]) {
    testWidgets(
      'compact link draft without an account ${dark ? 'dark' : 'light'}',
      (tester) async {
        await loadPreviewFonts();
        tester.view.physicalSize = const Size(390, 760);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        repository = MailtoRepositoryFixture(accounts: const []);
        workspace = Workspace(repository, MemorySettings());
        final draft = Draft(
          id: newDraftIdentity(),
          to: launchFields.to,
          cc: launchFields.cc,
          bcc: launchFields.bcc,
          subject: launchFields.subject,
          body: launchFields.body,
        );
        workspace.drafts[draft.id] = draft;
        await tester.pumpWidget(
          MaterialApp(
            debugShowCheckedModeBanner: false,
            theme: shepTheme(dark ? Brightness.dark : Brightness.light),
            home: Composer(workspace: workspace, draft: draft),
          ),
        );
        await tester.pumpAndSettle();
        await expectLater(
          find.byType(MaterialApp),
          matchesGoldenFile(
            'goldens/mailto_no_account_compact_${dark ? 'dark' : 'light'}.png',
          ),
        );
        await finish(tester);
      },
    );
  }

  testWidgets('a rejected link explains why and opens no draft', (
    tester,
  ) async {
    await start(tester);
    repository.failure = const MailOperationFailure(
      'This email link contains invalid encoded text, so Shep did not open it.',
    );
    links.arrive('mailto:a@example.test?subject=%FF');
    await settle(tester);
    expect(find.text('Email link not opened'), findsOneWidget);
    expect(find.textContaining('invalid encoded text'), findsOneWidget);
    await tester.tap(find.text('Close'));
    await settle(tester);
    expect(find.widgetWithText(AppBar, 'New message'), findsNothing);
    expect(workspace.drafts, isEmpty);
    await finish(tester);
  });

  testWidgets('a client without the native cache reports email links visibly', (
    tester,
  ) async {
    links.pending.add(launchLink);
    workspace = Workspace(
      PreviewRepository(delay: Duration.zero),
      MemorySettings(),
    );
    await tester.pumpWidget(ShepApp(workspace: workspace, links: links));
    await workspace.initialize();
    await settle(tester);
    expect(find.text('Email link not opened'), findsOneWidget);
    expect(find.textContaining('installed Shep app'), findsOneWidget);
    await tester.tap(find.text('Close'));
    await settle(tester);
    await finish(tester);
  });

  group('message links', () {
    Draft? result;
    Future<void> review(WidgetTester tester, String link) async {
      repository = MailtoRepositoryFixture()
        ..fields['mailto:friend%40example.test?subject=x&bcc=hidden@example.test'] =
            launchFields;
      workspace = Workspace(repository, MemorySettings());
      result = null;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () async =>
                    result = await reviewMessageLink(context, workspace, link),
                child: const Text('Follow fixture link'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Follow fixture link'));
      await settle(tester);
    }

    testWidgets('write a message from a link keeping only its address', (
      tester,
    ) async {
      const link =
          'mailto:friend%40example.test?subject=x&bcc=hidden@example.test';
      await review(tester, link);
      expect(find.text('Message link'), findsOneWidget);
      expect(find.text('Open link'), findsNothing);
      await tester.tap(find.text('Write message'));
      await settle(tester);
      expect(find.text('Message link'), findsNothing);
      final request = repository.requests.single;
      expect(request.message, isTrue);
      expect(request.link, link);
      expect(result?.to, 'friend@example.test');
      expect(result?.bcc, '');
      expect(result?.subject, '');
      expect(repository.sent, isEmpty);
      workspace.dispose();
    });

    testWidgets('a rejected message link keeps the review open to try again', (
      tester,
    ) async {
      await review(tester, 'mailto:%FF@example.test');
      repository.failure = const MailOperationFailure(
        'This email link contains invalid encoded text, so Shep did not open it.',
      );
      await tester.tap(find.text('Write message'));
      await settle(tester);
      expect(find.text('Message link'), findsOneWidget);
      expect(find.textContaining('invalid encoded text'), findsOneWidget);
      repository.failure = null;
      await tester.tap(find.text('Write message'));
      await settle(tester);
      expect(find.text('Message link'), findsNothing);
      expect(repository.requests, hasLength(2));
      expect(result, isNotNull);
      workspace.dispose();
    });

    testWidgets(
      'web links still open elsewhere and other schemes are ignored',
      (tester) async {
        await review(tester, 'https://example.test/help');
        expect(find.text('Open link'), findsOneWidget);
        expect(find.text('Write message'), findsNothing);
        await tester.tap(find.text('Close'));
        await settle(tester);
        await review(tester, 'javascript:alert(1)');
        expect(find.text('Message link'), findsNothing);
        expect(result, isNull);
        expect(repository.requests, isEmpty);
        workspace.dispose();
      },
    );
  });
}
