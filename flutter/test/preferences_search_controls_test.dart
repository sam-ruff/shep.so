import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/preferences_search.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/preferences.dart';
import 'package:shep_mobile/ui/preferences_catalogue.dart';
import 'package:shep_mobile/ui/theme.dart';

import 'account_connection_lifecycle_test.dart'
    show ConnectionRepository, account;
import 'preference_lifecycle_test.dart' show HeldSettings;
import 'calendar_lifecycle_test.dart' show CalDavCalendarRepository;
import 'support/google_fixture.dart';
import 'support/preview_repository.dart';
import 'support/profile_enrollment_fixture.dart';
import 'support/profile_sync_controls.dart' show mountSync;
import 'support/profile_sync_fixture.dart';
import 'workspace_test.dart' show MemorySettings;

class AccountSettingsRepository extends ConnectionRepository
    implements SentPreferencesRepository, AccountRemovalRepository {
  @override
  int pendingCredentialCleanup = 1;
  @override
  Future<void> cleanupCredentials() async => pendingCredentialCleanup = 0;
  @override
  Future<void> saveSentPreferences(
    String account,
    String policy,
    String folder,
  ) async {}
  @override
  Future<AccountRemoval> removalPreview(String id) async =>
      AccountRemoval({'id': id, 'email': account.email});
  @override
  Future<void> removeAccount(
    AccountRemoval review,
    bool discardUnresolved,
  ) async {}
}

class HeldPreferenceMatcher implements PreferenceSearchMatcher {
  final queries = <String>[];
  final replies = <Completer<List<PreferenceSearchEntry>>>[];
  @override
  Future<List<PreferenceSearchEntry>> match(
    List<PreferenceSearchEntry> entries,
    String query,
  ) {
    queries.add(query);
    final reply = Completer<List<PreferenceSearchEntry>>();
    replies.add(reply);
    return reply.future;
  }
}

Finder result(String target) => find.byWidgetPredicate(
  (widget) =>
      widget.key is ValueKey<String> &&
      (widget.key! as ValueKey<String>).value.startsWith(
        'preferences-result-$target-',
      ),
);

Future<void> search(WidgetTester tester, String text) async {
  await tester.enterText(
    find.byKey(const ValueKey('preferences-search')),
    text,
  );
  await tester.pump();
  await tester.pump();
}

Future<void> select(WidgetTester tester, String target) async {
  await tester.ensureVisible(result(target));
  await tester.tap(result(target));
  await tester.pump();
  await tester.pump();
}

Future<void> mount(WidgetTester tester, Workspace workspace) async {
  await tester.pumpWidget(
    MaterialApp(
      theme: shepTheme(Brightness.light),
      home: Scaffold(body: PreferencesView(workspace: workspace)),
    ),
  );
  await tester.pump();
}

void assertCatalogueControls(WidgetTester tester, Workspace workspace) {
  final entries = preferencesCatalogue(workspace);
  for (final entry in entries) {
    if (const {
      'caldav-url',
      'caldav-username',
      'caldav-password',
    }.contains(entry.target)) {
      continue;
    }
    expect(
      find.byKey(ValueKey(entry.target)),
      findsOneWidget,
      reason: '${entry.label} must reveal its real control',
    );
  }
  final root = find.byKey(const ValueKey('preferences-list'));
  for (final tile
      in find
          .descendant(of: root, matching: find.byType(ListTile))
          .evaluate()) {
    final widget = tile.widget as ListTile;
    if (widget.onTap == null && widget.trailing == null) continue;
    final keys = <Key?>[widget.key];
    void visit(Element element) {
      keys.add(element.widget.key);
      element.visitChildren(visit);
    }

    tile.visitChildren(visit);
    tile.visitAncestorElements((element) {
      keys.add(element.widget.key);
      return element.widget is! PreferencesView;
    });
    expect(
      entries.any((entry) => keys.contains(ValueKey(entry.target))),
      isTrue,
      reason: '${widget.title} is absent from search',
    );
    if (widget.title case Text(:final data?)) {
      final own = entries.where(
        (entry) => keys.contains(ValueKey(entry.target)),
      );
      if (own.isNotEmpty) {
        expect(
          searchPreferences(entries, data).any((entry) => own.contains(entry)),
          isTrue,
          reason: '$data must be searchable by its rendered caption',
        );
      }
    }
  }
  for (final tile
      in find
          .descendant(of: root, matching: find.byType(CheckboxListTile))
          .evaluate()) {
    expect(
      entries.any((entry) => ValueKey(entry.target) == tile.widget.key),
      isTrue,
      reason: '${tile.widget} is absent from search',
    );
  }
}

void main() {
  testWidgets(
    'search coalesces replacement queries, ignores late replies and owns Retry',
    (tester) async {
      final workspace = Workspace(PreviewRepository(), MemorySettings());
      addTearDown(workspace.dispose);
      final matcher = HeldPreferenceMatcher();
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: PreferencesView(workspace: workspace, matcher: matcher),
          ),
        ),
      );
      await search(tester, 'avatars');
      await search(tester, 'quoted');
      await search(tester, 'theme');
      expect(matcher.queries, ['avatars']);
      matcher.replies.first.complete([mobilePreferenceEntries[4]]);
      await tester.pump();
      expect(matcher.queries, ['avatars', 'theme']);
      expect(result('preference-avatars'), findsNothing);
      await tester.tap(find.byTooltip('Clear search'));
      await tester.pump();
      matcher.replies.last.complete([mobilePreferenceEntries.first]);
      await tester.pump();
      expect(find.byKey(const ValueKey('preferences-results')), findsNothing);
      await search(tester, 'theme');
      matcher.replies.last.completeError(StateError('private native failure'));
      await tester.pump();
      expect(find.text('Retry search'), findsOneWidget);
      expect(find.textContaining('private native failure'), findsNothing);
      await tester.tap(find.text('Retry search'));
      await tester.pump();
      matcher.replies.last.complete([mobilePreferenceEntries.first]);
      await tester.pump();
      expect(result('preference-theme'), findsOneWidget);
      expect(workspace.preferences.appearance, ThemeMode.system);
    },
  );
  testWidgets(
    'local catalogue covers actual controls, including offscreen settings',
    (tester) async {
      final workspace = Workspace(PreviewRepository(), MemorySettings());
      addTearDown(workspace.dispose);
      await mount(tester, workspace);
      assertCatalogueControls(tester, workspace);
      for (final entry in mobilePreferenceEntries) {
        await search(tester, entry.label);
        await select(tester, entry.target);
        expect(
          find.byKey(ValueKey(entry.target)).hitTestable(),
          findsOneWidget,
        );
      }
    },
  );

  testWidgets(
    'replaced catalogue identities reject a pending reply for the same query',
    (tester) async {
      final repository = ConnectionRepository();
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      final matcher = HeldPreferenceMatcher();
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: PreferencesView(workspace: workspace, matcher: matcher),
          ),
        ),
      );
      await search(tester, account.name);
      final older = preferencesCatalogue(workspace).firstWhere(
        (entry) => entry.target == 'preference-account-${account.id}',
      );
      repository.mailAccounts = [
        MailAccount.fromJson({...account.toJson(), 'id': 'replacement'}),
      ];
      await workspace.loadPage();
      await tester.pump();
      matcher.replies.first.complete([older]);
      await tester.pump();
      expect(matcher.queries, [account.name, account.name]);
      expect(result('preference-account-${account.id}'), findsNothing);
      final current = preferencesCatalogue(
        workspace,
      ).firstWhere((entry) => entry.target == 'preference-account-replacement');
      matcher.replies.last.complete([current]);
      await tester.pump();
      expect(result('preference-account-replacement'), findsOneWidget);
    },
  );

  for (final width in [320.0, 900.0]) {
    testWidgets(
      'search, no results, clear and revealed checkbox work at $width px',
      (tester) async {
        await tester.binding.setSurfaceSize(Size(width, 700));
        addTearDown(() => tester.binding.setSurfaceSize(null));
        final workspace = Workspace(PreviewRepository(), MemorySettings());
        addTearDown(workspace.dispose);
        await mount(tester, workspace);
        await search(tester, 'unfindable');
        expect(
          find.text('No preferences found. Try another word.'),
          findsOneWidget,
        );
        await tester.tap(find.byTooltip('Clear search'));
        await tester.pump();
        expect(
          find.byKey(const ValueKey('preference-theme')).hitTestable(),
          findsOneWidget,
        );
        await search(tester, 'avatars');
        await select(tester, 'preference-avatars');
        await tester.tap(find.byKey(const ValueKey('preference-avatars')));
        await tester.pump();
        expect(workspace.preferences.avatars, false);
        expect(tester.takeException(), isNull);
      },
    );
  }

  testWidgets(
    'search retains failed local intent and existing Retry ownership',
    (tester) async {
      final store = HeldSettings();
      final workspace = Workspace(PreviewRepository(), store);
      addTearDown(workspace.dispose);
      await mount(tester, workspace);
      await tester.tap(find.byType(DropdownButton<ThemeMode>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Dark').last);
      await tester.pumpAndSettle();
      store.writes.single.completeError(StateError('disk full'));
      await tester.pump();
      workspace.error = 'Unrelated mail failure';
      await search(tester, 'quoted');
      await select(tester, 'preference-quotes');
      expect(workspace.preferences.appearance, ThemeMode.dark);
      expect(workspace.preferenceSaveError, isNotNull);
      await search(tester, 'retry save');
      await select(tester, 'preference-save-status');
      await tester.tap(find.text('Retry save'));
      await tester.pump();
      store.writes.last.complete();
      await tester.pump();
      await tester.pump();
      expect(workspace.preferenceSaveError, isNull);
      expect(workspace.error, 'Unrelated mail failure');
      expect(store.value.appearance, ThemeMode.dark);
    },
  );

  testWidgets('new input cancels an older pending control reveal', (
    tester,
  ) async {
    final workspace = Workspace(PreviewRepository(), MemorySettings());
    addTearDown(workspace.dispose);
    await mount(tester, workspace);
    await search(tester, 'quoted');
    await tester.tap(result('preference-quotes'));
    await tester.enterText(
      find.byKey(const ValueKey('preferences-search')),
      'unfindable',
    );
    await tester.pump();
    await tester.pump();
    expect(
      find.text('No preferences found. Try another word.'),
      findsOneWidget,
    );
    expect(find.text('Showing: Quoted history'), findsNothing);
    expect(
      tester
          .widget<TextField>(find.byKey(const ValueKey('preferences-search')))
          .controller!
          .text,
      'unfindable',
    );
  });

  testWidgets(
    'conditional accounts reveal and open the existing setup destination',
    (tester) async {
      final repository = ConnectionRepository();
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await mount(tester, workspace);
      assertCatalogueControls(tester, workspace);
      await search(tester, account.email);
      await select(tester, 'preference-account-${account.id}');
      await tester.tap(
        find.byKey(ValueKey('preference-account-${account.id}')),
      );
      await tester.pumpAndSettle();
      expect(find.text('Reconnect account'), findsOneWidget);
    },
  );

  testWidgets(
    'Google results stay current during held consent without dispatching search',
    (tester) async {
      final sdk = FixtureGoogleAuthorization();
      final google = GoogleConnection(sdk, MemoryGoogleStore());
      await google.load();
      final workspace = Workspace(
        PreviewRepository(),
        MemorySettings(),
        google: google,
      );
      addTearDown(workspace.dispose);
      await mount(tester, workspace);
      assertCatalogueControls(tester, workspace);
      await search(tester, 'private drive');
      await select(tester, 'google-drive');
      await tester.tap(find.byKey(const ValueKey('google-drive')));
      await tester.pump();
      expect(google.requested.drive, isTrue);
      await search(tester, 'disconnect');
      expect(result('google-disconnect'), findsNothing);
      sdk.hold = Completer<void>();
      final pending = google.connect();
      await tester.pump();
      await search(tester, 'theme');
      await select(tester, 'preference-theme');
      expect(google.busy, isTrue);
      expect(sdk.connects, 1);
      sdk.hold!.complete();
      await pending;
      await search(tester, 'disconnect');
      await tester.pump();
      expect(result('google-disconnect'), findsOneWidget);
      await select(tester, 'google-disconnect');
      await tester.tap(find.byKey(const ValueKey('google-disconnect')));
      await tester.pumpAndSettle();
      expect(find.text('Disconnect Google here?'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(google.active, isNotNull);
    },
  );

  testWidgets(
    'conditional Sent, removal and cleanup search results use existing controls',
    (tester) async {
      final repository = AccountSettingsRepository();
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await mount(tester, workspace);
      assertCatalogueControls(tester, workspace);
      await search(tester, 'sent copies');
      await select(tester, 'preference-sent-${account.id}');
      await tester.tap(find.byKey(ValueKey('preference-sent-${account.id}')));
      await tester.pumpAndSettle();
      expect(find.text('Sent copies'), findsOneWidget);
      await tester.pageBack();
      await tester.pumpAndSettle();
      await search(tester, 'remove ${account.email}');
      await select(tester, 'preference-remove-${account.id}');
      await tester.tap(find.byKey(ValueKey('preference-remove-${account.id}')));
      await tester.pumpAndSettle();
      expect(find.text('Remove account'), findsOneWidget);
      await tester.pageBack();
      await tester.pumpAndSettle();
      await search(tester, 'retry cleanup');
      await select(tester, 'preference-credential-cleanup');
      await tester.tap(find.text('Retry cleanup'));
      await tester.pumpAndSettle();
      expect(repository.pendingCredentialCleanup, 0);
      await search(tester, 'retry cleanup');
      expect(result('preference-credential-cleanup'), findsNothing);
    },
  );

  testWidgets(
    'profile and per-field sync controls enter the catalogue only when available',
    (tester) async {
      final fixture = FixtureProfileSync(
        EnrollmentMail(),
        accounts: 0,
        completed: true,
      );
      final (workspace, discovery, cleanup) = await mountSync(tester, fixture);
      assertCatalogueControls(tester, workspace);
      await search(tester, 'keep in sync');
      await select(tester, 'profile-sync-subscribe');
      await tester.tap(find.byKey(const ValueKey('profile-sync-subscribe')));
      await tester.pumpAndSettle();
      assertCatalogueControls(tester, workspace);
      await search(tester, 'sync tooltips');
      await select(tester, 'profile-sync-field-tooltips');
      expect(
        find.byKey(const ValueKey('profile-sync-field-tooltips')).hitTestable(),
        findsOneWidget,
      );
      await search(tester, 'sync preferences');
      await select(tester, 'profile-sync-master');
      await tester.tap(find.byKey(const ValueKey('profile-sync-master')));
      await tester.pumpAndSettle();
      expect(discovery.sync!.enabled, isTrue);
      await search(tester, 'saved google profiles');
      await select(tester, 'preference-profiles');
      await tester.tap(find.byKey(const ValueKey('preference-profiles')));
      await tester.pumpAndSettle();
      expect(find.text('Profiles and sync'), findsOneWidget);
      cleanup();
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
  testWidgets(
    'CalDAV field reveal preserves entered credentials through search',
    (tester) async {
      final repository = CalDavCalendarRepository();
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await mount(tester, workspace);
      assertCatalogueControls(tester, workspace);
      await search(tester, 'calendar url');
      await select(tester, 'caldav-url');
      expect(
        tester
            .widget<EditableText>(
              find.descendant(
                of: find.byKey(const ValueKey('caldav-url')),
                matching: find.byType(EditableText),
              ),
            )
            .focusNode
            .hasFocus,
        isTrue,
      );
      await tester.enterText(
        find.byKey(const ValueKey('caldav-url')),
        'https://calendar.example.test/new/',
      );
      await search(tester, 'caldav password');
      await select(tester, 'caldav-password');
      await tester.enterText(
        find.byKey(const ValueKey('caldav-password')),
        'transient-test-password',
      );
      await search(tester, 'avatars');
      await select(tester, 'preference-avatars');
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('caldav-url')))
            .controller!
            .text,
        'https://calendar.example.test/new/',
      );
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('caldav-password')))
            .controller!
            .text,
        'transient-test-password',
      );
      expect(repository.savedPasswords, isEmpty);
    },
  );
  testWidgets('conditional CalDAV cleanup search reveals the existing Retry', (
    tester,
  ) async {
    final repository = CalDavCalendarRepository()..failCleanup = true;
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await mount(tester, workspace);
    await tester.pump();
    await tester.pump();
    assertCatalogueControls(tester, workspace);
    await search(tester, 'credential cleanup waiting');
    await select(tester, 'caldav-cleanup');
    repository.failCleanup = false;
    await tester.tap(find.text('Retry cleanup'));
    await tester.pumpAndSettle();
    expect(workspace.calDavCleanupError, isNull);
    await search(tester, 'credential cleanup waiting');
    expect(result('caldav-cleanup'), findsNothing);
  });
}
