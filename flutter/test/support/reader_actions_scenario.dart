import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/formatted_message.dart';
import 'package:shep_mobile/data/printing.dart';
import 'package:shep_mobile/data/drafts.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import '../workspace_test.dart' show MemorySettings;
import 'paged_repository.dart';

// Only transport/preparation are gated. All user actions enter real controls.
class ReaderActionsRepository extends PagedRepository
    implements FormattedMessageRepository, PrintRepository, ForwardRepository {
  final body = Completer<Mail>();
  final document = Completer<PreparedMessage>();
  final prints = <Completer<PreparedPrint>>[];
  final forwards = <Completer<Draft>>[];
  @override
  Future<Draft> forward(String id, String draftId) {
    final result = Completer<Draft>();
    forwards.add(result);
    return result.future;
  }

  @override
  Future<Mail> detail(String id) => body.future;
  @override
  Future<PreparedMessage> formattedMessage(
    String id, {
    required String generation,
    required bool dark,
    required bool quotes,
  }) => document.future;
  @override
  Future<PreparedPrint> preparePrint(
    String id, {
    required String generation,
    required bool plain,
  }) {
    final result = Completer<PreparedPrint>();
    prints.add(result);
    return result.future;
  }
}

class ReaderActionsPrinter implements MessagePrinter {
  final opened = <PreparedPrint>[];
  @override
  Future<void> open(
    PreparedPrint prepared, {
    required String generation,
  }) async {
    opened.add(prepared);
  }
}

Future<void> readerActionsScenario(
  WidgetTester tester, {
  required bool dark,
  Future<void> Function(String)? capture,
}) async {
  final repo = ReaderActionsRepository();
  final printer = ReaderActionsPrinter();
  final settings = MemorySettings();
  settings.value = settings.value.copy(
    appearance: dark ? ThemeMode.dark : ThemeMode.light,
  );
  final workspace = Workspace(repo, settings, printer: printer);
  await workspace.initialize();
  workspace.setForeground(false);
  try {
    await tester.pumpWidget(ShepApp(workspace: workspace));
    await tester.pumpAndSettle();
    await tester.tap(find.text('A little room for good ideas'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    await tester.pump();

    Finder action(String name) => find.ancestor(
      of: find.text(name),
      matching: find.byWidgetPredicate((w) => w is ButtonStyleButton),
    );
    final route = ModalRoute.of(tester.element(action('Print')))!;
    for (
      var i = 0;
      i < 20 && route.animation!.status != AnimationStatus.completed;
      i++
    ) {
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(route.animation!.status, AnimationStatus.completed);
    final labels = ['Reply', 'Reply all', 'Forward', 'Print', 'Move'];
    final before = {
      for (final name in labels) name: tester.getRect(action(name)),
    };
    final screen = tester.view.physicalSize / tester.view.devicePixelRatio;
    for (final name in labels) {
      final rect = before[name]!;
      expect(rect.top, greaterThan(50));
      expect(
        rect.bottom,
        lessThanOrEqualTo(
          screen.height -
              tester.view.padding.bottom / tester.view.devicePixelRatio,
        ),
      );
      expect(rect.height, greaterThanOrEqualTo(44));
      expect(action(name).hitTestable(), findsOneWidget);
    }
    expect(tester.widget<ButtonStyleButton>(action('Reply')).enabled, false);
    final printElement = tester.element(action('Print'));
    final press = await tester.startGesture(before['Print']!.center);

    final original = repo.cached.firstWhere((m) => m.id == '1');
    final longText = List.generate(
      60,
      (i) => 'Prepared message paragraph $i.',
    ).join('\n\n');
    repo.body.complete(
      original.withDetail(
        Mail(
          id: original.id,
          sender: original.sender,
          address: original.address,
          subject: original.subject,
          preview: original.preview,
          body: longText,
          date: original.date,
          attachments: List.generate(8, (i) => 'Arrival $i.pdf'),
        ),
      ),
    );
    repo.document.complete(
      PreparedMessage(signature: 'delayed-reader', text: longText),
    );
    // No settle while loading/preparation progress can still animate.
    await tester.pump(const Duration(milliseconds: 100));
    await tester.pump(const Duration(milliseconds: 100));
    expect(workspace.mail('1')!.bodyLoaded, true);
    expect(tester.element(action('Print')), same(printElement));
    for (final name in labels) {
      expect(tester.getRect(action(name)), before[name]);
    }
    await press.up();
    await tester.pump(const Duration(milliseconds: 100));
    expect(repo.prints, hasLength(1));
    expect(tester.widget<ButtonStyleButton>(action('Print')).enabled, false);
    expect(find.text('Print'), findsOneWidget);
    expect(tester.getRect(action('Print')), before['Print']);
    expect(printer.opened, isEmpty);
    await tester.tap(action('Print'));
    await tester.pump();
    expect(repo.prints, hasLength(1));
    repo.prints.single.complete(
      const PreparedPrint(
        document: 'Complete source',
        title: 'Reader',
        signature: 'delayed-reader',
        accountId: 'work',
      ),
    );
    await tester.pumpAndSettle();
    expect(printer.opened, hasLength(1));
    expect(tester.widget<ButtonStyleButton>(action('Print')).enabled, true);

    await tester.tap(action('Forward'));
    await tester.pump(const Duration(milliseconds: 100));
    expect(repo.forwards, hasLength(1));
    expect(tester.widget<ButtonStyleButton>(action('Forward')).enabled, false);
    for (final name in labels) {
      expect(tester.getRect(action(name)), before[name]);
    }
    repo.forwards.single.completeError(
      StateError('Synthetic preparation failure. Retry Forward.'),
    );
    await tester.pumpAndSettle();
    expect(find.textContaining('Retry Forward.'), findsOneWidget);
    expect(tester.widget<ButtonStyleButton>(action('Forward')).enabled, true);
    expect(tester.getRect(action('Forward')), before['Forward']);
    await tester.tap(find.byTooltip('Dismiss error'));
    await tester.pumpAndSettle();

    await tester.drag(
      find.byType(SingleChildScrollView).last,
      const Offset(0, -450),
    );
    await tester.pumpAndSettle();
    for (final name in labels) {
      expect(tester.getRect(action(name)), before[name]);
      expect(action(name).hitTestable(), findsOneWidget);
    }
    await capture?.call('native-reader-footer-${dark ? 'dark' : 'light'}');
    await tester.tap(action('Reply'));
    await tester.pumpAndSettle();
    expect(find.text('Re: A little room for good ideas'), findsOneWidget);
    expect(find.text(original.address), findsOneWidget);
    for (final job in repo.jobs) {
      if (!job.isCompleted) job.complete();
    }
    await tester.pumpAndSettle();
  } finally {
    workspace.dispose();
    await tester.pumpWidget(const SizedBox());
  }
}
