import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/ui/format.dart';
import 'package:shep_mobile/ui/icons.dart';
import 'package:shep_mobile/ui/theme.dart';

/// Desktop palette from src/ui/components.rs; both clients must keep it.
const desktopLight = {
  'bg': 0xfff7f7f9,
  'surface': 0xffffffff,
  'subtle': 0xfff4f4f7,
  'text': 0xff292830,
  'muted': 0xff777580,
  'border': 0xffe8e7ed,
  'accent': 0xff7356bd,
  'tint': 0xfff0eafa,
  'flag': 0xffc62828,
};
const desktopDark = {
  'bg': 0xff141416,
  'surface': 0xff1b1b1f,
  'subtle': 0xff232329,
  'text': 0xfff4f4f5,
  'muted': 0xffa5a5b0,
  'border': 0xff323239,
  'accent': 0xffb5a0ff,
  'tint': 0xff30283f,
  'flag': 0xfff87171,
};

Map<String, int> tokens(ShepColors c) => {
  'bg': c.bg.toARGB32(),
  'surface': c.surface.toARGB32(),
  'subtle': c.subtle.toARGB32(),
  'text': c.text.toARGB32(),
  'muted': c.muted.toARGB32(),
  'border': c.border.toARGB32(),
  'accent': c.accent.toARGB32(),
  'tint': c.tint.toARGB32(),
  'flag': c.flag.toARGB32(),
};

void main() {
  test('palette tokens match the desktop client', () {
    expect(tokens(ShepColors.light), desktopLight);
    expect(tokens(ShepColors.dark), desktopDark);
    expect(ShepColors.primaryButton.toARGB32(), 0xff7356bd);
  });

  test('theme maps desktop tokens onto the Material colour scheme', () {
    for (final brightness in Brightness.values) {
      final theme = shepTheme(brightness);
      final c = theme.extension<ShepColors>()!;
      expect(theme.colorScheme.primary, c.accent);
      expect(theme.colorScheme.surface, c.surface);
      expect(theme.colorScheme.onSurface, c.text);
      expect(theme.colorScheme.onSurfaceVariant, c.muted);
      expect(theme.colorScheme.outlineVariant, c.border);
      expect(theme.colorScheme.primaryContainer, c.tint);
      expect(theme.colorScheme.error, c.flag);
      expect(theme.scaffoldBackgroundColor, c.bg);
      expect(theme.textTheme.bodyMedium?.fontSize, ShepText.body);
      expect(theme.textTheme.headlineSmall?.fontSize, ShepText.heading);
      expect(theme.textTheme.bodyMedium?.fontFamily, 'NotoSans');
    }
  });

  test('primary buttons keep the desktop violet on both schemes', () {
    for (final brightness in Brightness.values) {
      final style = shepTheme(brightness).filledButtonTheme.style!;
      expect(style.backgroundColor!.resolve({}), ShepColors.primaryButton);
      expect(style.foregroundColor!.resolve({}), Colors.white);
      final shape = style.shape!.resolve({}) as RoundedRectangleBorder;
      expect(shape.borderRadius, BorderRadius.circular(ShepRadius.control));
    }
  });

  test('desktop icon names resolve and mail actions map to them', () {
    for (final name in ['flag', 'archive', 'trash', 'compose', 'settings']) {
      expect(shepIconShapes.containsKey(name), isTrue, reason: name);
    }
    expect(actionIconName('archive'), 'archive');
    expect(actionIconName('read'), 'mail-open');
    expect(actionIconName('star'), 'flag');
  });

  testWidgets('icons paint without throwing on every shape', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: shepTheme(Brightness.light),
        home: Wrap(
          children: [
            for (final name in shepIconShapes.keys) ShepIcon(name, size: 24),
          ],
        ),
      ),
    );
    expect(find.byType(ShepIcon), findsNWidgets(shepIconShapes.length));
    expect(tester.takeException(), isNull);
  });

  test('dates follow the desktop row and reader formats', () {
    final now = DateTime(2026, 9, 9, 15, 4);
    expect(rowDate(DateTime(2026, 9, 9, 9, 5), now: now), '09:05');
    expect(rowDate(DateTime(2026, 9, 6, 10, 42), now: now), '06 Sep');
    expect(readerDate(DateTime(2026, 9, 9, 15, 4)), '09 Sep 2026\n15:04');
  });

  test('avatars use the desktop initials and palette cycle', () {
    expect(avatarInitials('Sophie Williams'), 'SW');
    expect(avatarInitials('Figma'), 'F');
    expect(avatarColors(0), avatarColors(5));
    expect(avatarColors(1).$1.toARGB32(), 0xffe5eaf5);
  });
}
