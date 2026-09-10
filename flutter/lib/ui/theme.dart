import 'package:flutter/material.dart';
import 'icons.dart';

/// Desktop palette tokens (src/ui/components.rs `Colors`), exposed to widgets.
class ShepColors extends ThemeExtension<ShepColors> {
  const ShepColors({
    required this.bg,
    required this.surface,
    required this.subtle,
    required this.text,
    required this.muted,
    required this.border,
    required this.accent,
    required this.tint,
    required this.flag,
    required this.errorSurface,
  });

  static const light = ShepColors(
    bg: Color(0xfff7f7f9),
    surface: Color(0xffffffff),
    subtle: Color(0xfff4f4f7),
    text: Color(0xff292830),
    muted: Color(0xff777580),
    border: Color(0xffe8e7ed),
    accent: Color(0xff7356bd),
    tint: Color(0xfff0eafa),
    flag: Color(0xffc62828),
    errorSurface: Color(0xfffff0ef),
  );

  static const dark = ShepColors(
    bg: Color(0xff141416),
    surface: Color(0xff1b1b1f),
    subtle: Color(0xff232329),
    text: Color(0xfff4f4f5),
    muted: Color(0xffa5a5b0),
    border: Color(0xff323239),
    accent: Color(0xffb5a0ff),
    tint: Color(0xff30283f),
    flag: Color(0xfff87171),
    errorSurface: Color(0xff382325),
  );

  /// Primary buttons keep the same violet on both schemes, like the desktop.
  static const primaryButton = Color(0xff7356bd);
  static const primaryButtonHover = Color(0xff8060cc);
  static const primaryButtonPressed = Color(0xff60459f);
  static const destructive = Color(0xffb91c1c);

  /// Avatar (background, foreground) pairs cycled by sender index.
  static const avatarPalettes = [
    (Color(0xffeee5e2), Color(0xff9b6960)),
    (Color(0xffe5eaf5), Color(0xff5976a1)),
    (Color(0xffe8eadd), Color(0xff778454)),
    (Color(0xfff1e7f4), Color(0xff9a69a7)),
    (Color(0xfff5eddc), Color(0xffa18b52)),
  ];

  final Color bg;
  final Color surface;
  final Color subtle;
  final Color text;
  final Color muted;
  final Color border;
  final Color accent;
  final Color tint;
  final Color flag;
  final Color errorSurface;

  static ShepColors of(BuildContext context) =>
      Theme.of(context).extension<ShepColors>() ??
      (Theme.of(context).brightness == Brightness.dark ? dark : light);

  @override
  ShepColors copyWith({
    Color? bg,
    Color? surface,
    Color? subtle,
    Color? text,
    Color? muted,
    Color? border,
    Color? accent,
    Color? tint,
    Color? flag,
    Color? errorSurface,
  }) => ShepColors(
    bg: bg ?? this.bg,
    surface: surface ?? this.surface,
    subtle: subtle ?? this.subtle,
    text: text ?? this.text,
    muted: muted ?? this.muted,
    border: border ?? this.border,
    accent: accent ?? this.accent,
    tint: tint ?? this.tint,
    flag: flag ?? this.flag,
    errorSurface: errorSurface ?? this.errorSurface,
  );

  @override
  ShepColors lerp(ShepColors? other, double t) {
    if (other == null) return this;
    Color mix(Color a, Color b) => Color.lerp(a, b, t) ?? b;
    return ShepColors(
      bg: mix(bg, other.bg),
      surface: mix(surface, other.surface),
      subtle: mix(subtle, other.subtle),
      text: mix(text, other.text),
      muted: mix(muted, other.muted),
      border: mix(border, other.border),
      accent: mix(accent, other.accent),
      tint: mix(tint, other.tint),
      flag: mix(flag, other.flag),
      errorSurface: mix(errorSurface, other.errorSurface),
    );
  }
}

/// Desktop radii: cards 12, buttons and fields 8, ghost controls and pick lists 7, badges 5.
abstract final class ShepRadius {
  static const card = 12.0;
  static const control = 8.0;
  static const ghost = 7.0;
  static const badge = 5.0;
}

/// Desktop type scale in logical pixels.
abstract final class ShepText {
  static const heading = 23.0;
  static const dialogTitle = 21.0;
  static const cardTitle = 16.0;
  static const body = 13.0;
  static const reader = 14.0;
  static const secondary = 12.0;
  static const small = 11.0;
  static const caption = 10.0;
}

/// Avatar colours for a sender, cycling the desktop palette by list index.
(Color, Color) avatarColors(int index) =>
    ShepColors.avatarPalettes[index % ShepColors.avatarPalettes.length];

String avatarInitials(String name) => name
    .split(RegExp(r'\s+'))
    .where((w) => w.isNotEmpty)
    .take(2)
    .map((w) => w[0])
    .join()
    .toUpperCase();

/// Rounded field outline drawn below the floating label so the label sits
/// above the box like the desktop form fields.
class ShepInputBorder extends InputBorder {
  const ShepInputBorder({
    super.borderSide = const BorderSide(),
    this.radius = ShepRadius.control,
    this.labelHeight = 16,
  });
  final double radius;
  final double labelHeight;

  @override
  bool get isOutline => true;

  @override
  EdgeInsetsGeometry get dimensions => EdgeInsets.all(borderSide.width);

  @override
  ShepInputBorder copyWith({
    BorderSide? borderSide,
    double? radius,
    double? labelHeight,
  }) => ShepInputBorder(
    borderSide: borderSide ?? this.borderSide,
    radius: radius ?? this.radius,
    labelHeight: labelHeight ?? this.labelHeight,
  );

  @override
  ShapeBorder scale(double t) =>
      ShepInputBorder(borderSide: borderSide.scale(t), radius: radius * t);

  RRect _box(Rect rect) => RRect.fromRectAndRadius(
    Rect.fromLTRB(
      rect.left,
      rect.top + labelHeight / 2,
      rect.right,
      rect.bottom,
    ),
    Radius.circular(radius),
  );

  @override
  Path getInnerPath(Rect rect, {TextDirection? textDirection}) =>
      Path()..addRRect(_box(rect).deflate(borderSide.width));

  @override
  Path getOuterPath(Rect rect, {TextDirection? textDirection}) =>
      Path()..addRRect(_box(rect));

  @override
  void paint(
    Canvas canvas,
    Rect rect, {
    double? gapStart,
    double gapExtent = 0.0,
    double gapPercentage = 0.0,
    TextDirection? textDirection,
  }) {
    if (borderSide.style == BorderStyle.none) return;
    canvas.drawRRect(
      _box(rect).deflate(borderSide.width / 2),
      borderSide.toPaint(),
    );
  }

  @override
  bool get preferPaintInterior => true;

  @override
  void paintInterior(
    Canvas canvas,
    Rect rect,
    Paint paint, {
    TextDirection? textDirection,
  }) => canvas.drawRRect(_box(rect), paint);

  @override
  bool operator ==(Object other) =>
      other is ShepInputBorder &&
      other.borderSide == borderSide &&
      other.radius == radius &&
      other.labelHeight == labelHeight;

  @override
  int get hashCode => Object.hash(borderSide, radius, labelHeight);
}

ThemeData shepTheme(Brightness brightness) {
  final dark = brightness == Brightness.dark;
  final c = dark ? ShepColors.dark : ShepColors.light;
  final scheme = ColorScheme(
    brightness: brightness,
    primary: c.accent,
    onPrimary: dark ? c.bg : c.surface,
    primaryContainer: c.tint,
    onPrimaryContainer: c.accent,
    secondary: c.accent,
    onSecondary: dark ? c.bg : c.surface,
    secondaryContainer: c.tint,
    onSecondaryContainer: c.accent,
    tertiary: c.accent,
    onTertiary: dark ? c.bg : c.surface,
    tertiaryContainer: c.tint,
    onTertiaryContainer: c.accent,
    error: c.flag,
    onError: Colors.white,
    errorContainer: c.errorSurface,
    onErrorContainer: c.text,
    surface: c.surface,
    onSurface: c.text,
    onSurfaceVariant: c.muted,
    surfaceContainerLowest: c.surface,
    surfaceContainerLow: c.subtle,
    surfaceContainer: c.subtle,
    surfaceContainerHigh: c.subtle,
    surfaceContainerHighest: c.subtle,
    surfaceDim: c.bg,
    surfaceBright: c.surface,
    outline: c.border,
    outlineVariant: c.border,
    shadow: Colors.black,
    scrim: const Color(0x73141420),
    inverseSurface: dark ? c.text : c.bg,
    onInverseSurface: dark ? c.bg : c.text,
    inversePrimary: dark ? ShepColors.light.accent : ShepColors.dark.accent,
    surfaceTint: Colors.transparent,
  );
  const family = 'NotoSans';
  TextStyle style(
    double size, {
    FontWeight weight = FontWeight.w400,
    Color? color,
  }) => TextStyle(
    fontFamily: family,
    fontSize: size,
    fontWeight: weight,
    color: color ?? c.text,
    height: 1.4,
  );
  final textTheme = TextTheme(
    displaySmall: style(27, weight: FontWeight.w600),
    headlineMedium: style(27, weight: FontWeight.w600),
    headlineSmall: style(ShepText.heading, weight: FontWeight.w600),
    titleLarge: style(ShepText.dialogTitle, weight: FontWeight.w600),
    titleMedium: style(ShepText.cardTitle, weight: FontWeight.w600),
    titleSmall: style(ShepText.body, weight: FontWeight.w600),
    bodyLarge: style(ShepText.reader),
    bodyMedium: style(ShepText.body),
    bodySmall: style(ShepText.secondary, color: c.muted),
    labelLarge: style(ShepText.body),
    labelMedium: style(ShepText.secondary),
    labelSmall: style(
      ShepText.caption,
      color: c.muted,
    ).copyWith(letterSpacing: 0),
  );
  final controlShape = RoundedRectangleBorder(
    borderRadius: BorderRadius.circular(ShepRadius.control),
  );
  final ghostShape = RoundedRectangleBorder(
    borderRadius: BorderRadius.circular(ShepRadius.ghost),
  );
  final cardShape = RoundedRectangleBorder(
    borderRadius: BorderRadius.circular(ShepRadius.card),
    side: BorderSide(color: c.border),
  );
  final fieldBorder = ShepInputBorder(borderSide: BorderSide(color: c.border));
  return ThemeData(
    useMaterial3: true,
    brightness: brightness,
    colorScheme: scheme,
    extensions: [c],
    fontFamily: family,
    textTheme: textTheme,
    scaffoldBackgroundColor: c.bg,
    canvasColor: c.surface,
    dividerColor: c.border,
    splashFactory: NoSplash.splashFactory,
    iconTheme: IconThemeData(color: c.muted, size: 20),
    actionIconTheme: ActionIconThemeData(
      backButtonIconBuilder: (_) => const ShepIcon('left', size: 20),
      closeButtonIconBuilder: (_) => const ShepIcon('close', size: 20),
      drawerButtonIconBuilder: (_) => const ShepIcon('menu', size: 20),
      endDrawerButtonIconBuilder: (_) => const ShepIcon('menu', size: 20),
    ),
    appBarTheme: AppBarTheme(
      backgroundColor: c.bg,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      scrolledUnderElevation: 0,
      centerTitle: false,
      iconTheme: IconThemeData(color: c.muted, size: 20),
      actionsIconTheme: IconThemeData(color: c.muted, size: 20),
      titleTextStyle: style(ShepText.heading, weight: FontWeight.w600),
    ),
    drawerTheme: DrawerThemeData(
      backgroundColor: c.bg,
      surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(
        side: BorderSide(color: c.border),
        borderRadius: BorderRadius.zero,
      ),
    ),
    listTileTheme: ListTileThemeData(
      iconColor: c.muted,
      textColor: c.text,
      titleTextStyle: style(ShepText.body),
      subtitleTextStyle: style(ShepText.secondary, color: c.muted),
      selectedColor: c.accent,
      selectedTileColor: c.tint,
      shape: controlShape,
      contentPadding: const EdgeInsets.symmetric(horizontal: 16),
      minVerticalPadding: 10,
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: ButtonStyle(
        minimumSize: const WidgetStatePropertyAll(Size(0, 44)),
        padding: const WidgetStatePropertyAll(
          EdgeInsets.symmetric(horizontal: 14, vertical: 10),
        ),
        foregroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.disabled) ? c.muted : c.text,
        ),
        iconColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.disabled) ? c.muted : c.text,
        ),
        iconSize: const WidgetStatePropertyAll(18),
        backgroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.pressed)
              ? c.tint
              : s.contains(WidgetState.hovered)
              ? c.subtle
              : c.surface,
        ),
        overlayColor: const WidgetStatePropertyAll(Colors.transparent),
        side: WidgetStatePropertyAll(BorderSide(color: c.border)),
        shape: WidgetStatePropertyAll(controlShape),
        textStyle: WidgetStatePropertyAll(style(ShepText.body)),
        elevation: const WidgetStatePropertyAll(0),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: ButtonStyle(
        minimumSize: const WidgetStatePropertyAll(Size(0, 44)),
        padding: const WidgetStatePropertyAll(
          EdgeInsets.symmetric(horizontal: 14, vertical: 10),
        ),
        foregroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.disabled) ? c.muted : Colors.white,
        ),
        iconColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.disabled) ? c.muted : Colors.white,
        ),
        iconSize: const WidgetStatePropertyAll(18),
        backgroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.disabled)
              ? c.subtle
              : s.contains(WidgetState.pressed)
              ? ShepColors.primaryButtonPressed
              : s.contains(WidgetState.hovered)
              ? ShepColors.primaryButtonHover
              : ShepColors.primaryButton,
        ),
        overlayColor: const WidgetStatePropertyAll(Colors.transparent),
        shape: WidgetStatePropertyAll(controlShape),
        textStyle: WidgetStatePropertyAll(style(ShepText.body)),
        elevation: const WidgetStatePropertyAll(0),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: ButtonStyle(
        minimumSize: const WidgetStatePropertyAll(Size(0, 44)),
        padding: const WidgetStatePropertyAll(
          EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        ),
        foregroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.disabled) ? c.muted : c.text,
        ),
        iconColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.disabled) ? c.muted : c.muted,
        ),
        iconSize: const WidgetStatePropertyAll(18),
        backgroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.pressed)
              ? c.tint
              : s.contains(WidgetState.hovered)
              ? c.subtle
              : Colors.transparent,
        ),
        overlayColor: const WidgetStatePropertyAll(Colors.transparent),
        shape: WidgetStatePropertyAll(ghostShape),
        textStyle: WidgetStatePropertyAll(style(ShepText.body)),
      ),
    ),
    iconButtonTheme: IconButtonThemeData(
      style: ButtonStyle(
        foregroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.disabled)
              ? c.muted.withValues(alpha: .5)
              : c.muted,
        ),
        backgroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.pressed)
              ? c.tint
              : s.contains(WidgetState.hovered)
              ? c.subtle
              : Colors.transparent,
        ),
        overlayColor: const WidgetStatePropertyAll(Colors.transparent),
        iconSize: const WidgetStatePropertyAll(20),
        shape: WidgetStatePropertyAll(ghostShape),
      ),
    ),
    segmentedButtonTheme: SegmentedButtonThemeData(
      style: ButtonStyle(
        side: WidgetStatePropertyAll(BorderSide(color: c.border)),
        shape: WidgetStatePropertyAll(controlShape),
        textStyle: WidgetStatePropertyAll(style(ShepText.secondary)),
        backgroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.selected) ? c.tint : c.surface,
        ),
        foregroundColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.selected) ? c.accent : c.text,
        ),
        overlayColor: const WidgetStatePropertyAll(Colors.transparent),
        visualDensity: VisualDensity.compact,
      ),
    ),
    dividerTheme: DividerThemeData(color: c.border, thickness: 1, space: 1),
    cardTheme: CardThemeData(
      elevation: 0,
      color: c.surface,
      surfaceTintColor: Colors.transparent,
      shape: cardShape,
      margin: EdgeInsets.zero,
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: c.surface,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      shape: cardShape,
      titleTextStyle: style(ShepText.dialogTitle, weight: FontWeight.w600),
      contentTextStyle: style(ShepText.body),
    ),
    popupMenuTheme: PopupMenuThemeData(
      color: c.surface,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      shape: cardShape,
      textStyle: style(ShepText.body),
      iconColor: c.muted,
    ),
    dropdownMenuTheme: DropdownMenuThemeData(
      textStyle: style(ShepText.secondary),
      menuStyle: MenuStyle(
        backgroundColor: WidgetStatePropertyAll(c.surface),
        surfaceTintColor: const WidgetStatePropertyAll(Colors.transparent),
        elevation: const WidgetStatePropertyAll(0),
        shape: WidgetStatePropertyAll(
          controlShape.copyWith(side: BorderSide(color: c.border)),
        ),
      ),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: c.surface,
      border: fieldBorder,
      enabledBorder: fieldBorder,
      disabledBorder: fieldBorder,
      focusedBorder: ShepInputBorder(borderSide: BorderSide(color: c.accent)),
      errorBorder: ShepInputBorder(borderSide: BorderSide(color: c.flag)),
      focusedErrorBorder: ShepInputBorder(
        borderSide: BorderSide(color: c.flag),
      ),
      floatingLabelBehavior: FloatingLabelBehavior.always,
      labelStyle: style(ShepText.body, weight: FontWeight.w600),
      // Floating labels render at 0.75 scale, so 16 lands on the 12px desktop label.
      floatingLabelStyle: style(16, weight: FontWeight.w600),
      hintStyle: style(ShepText.body, color: c.muted),
      helperStyle: style(ShepText.secondary, color: c.muted),
      errorStyle: style(ShepText.secondary, color: c.flag),
      prefixIconColor: c.muted,
      suffixIconColor: c.muted,
      contentPadding: const EdgeInsets.fromLTRB(12, 19, 12, 11),
    ),
    checkboxTheme: CheckboxThemeData(
      fillColor: WidgetStateProperty.resolveWith(
        (s) => s.contains(WidgetState.selected) ? c.accent : c.surface,
      ),
      checkColor: WidgetStatePropertyAll(dark ? c.bg : Colors.white),
      side: BorderSide(color: c.border),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(4)),
      overlayColor: const WidgetStatePropertyAll(Colors.transparent),
      visualDensity: VisualDensity.compact,
    ),
    switchTheme: SwitchThemeData(
      thumbColor: WidgetStatePropertyAll(dark ? c.bg : Colors.white),
      trackColor: WidgetStateProperty.resolveWith(
        (s) => s.contains(WidgetState.selected) ? c.accent : c.border,
      ),
      trackOutlineColor: const WidgetStatePropertyAll(Colors.transparent),
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: c.bg,
      surfaceTintColor: Colors.transparent,
      indicatorColor: c.tint,
      indicatorShape: controlShape,
      elevation: 0,
      height: 64,
      iconTheme: WidgetStateProperty.resolveWith(
        (s) => IconThemeData(
          size: 20,
          color: s.contains(WidgetState.selected) ? c.accent : c.muted,
        ),
      ),
      labelTextStyle: WidgetStateProperty.resolveWith(
        (s) => style(
          ShepText.small,
          color: s.contains(WidgetState.selected) ? c.accent : c.muted,
        ),
      ),
    ),
    floatingActionButtonTheme: FloatingActionButtonThemeData(
      backgroundColor: ShepColors.primaryButton,
      foregroundColor: Colors.white,
      hoverColor: ShepColors.primaryButtonHover,
      splashColor: ShepColors.primaryButtonPressed,
      elevation: 1,
      focusElevation: 1,
      hoverElevation: 1,
      highlightElevation: 1,
      shape: controlShape,
      extendedPadding: const EdgeInsets.symmetric(horizontal: 14),
      extendedTextStyle: style(ShepText.body, color: Colors.white),
      extendedIconLabelSpacing: 10,
    ),
    chipTheme: ChipThemeData(
      backgroundColor: c.surface,
      selectedColor: c.tint,
      side: BorderSide(color: c.border),
      shape: ghostShape,
      labelStyle: style(ShepText.secondary),
      secondaryLabelStyle: style(ShepText.secondary, color: c.accent),
      iconTheme: IconThemeData(color: c.muted, size: 16),
      deleteIconColor: c.muted,
      checkmarkColor: c.accent,
      showCheckmark: false,
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
      labelPadding: const EdgeInsets.symmetric(horizontal: 2),
      elevation: 0,
      pressElevation: 0,
    ),
    snackBarTheme: SnackBarThemeData(
      backgroundColor: c.surface,
      contentTextStyle: style(ShepText.body),
      shape: cardShape,
      elevation: 0,
      behavior: SnackBarBehavior.floating,
    ),
    progressIndicatorTheme: ProgressIndicatorThemeData(
      color: c.accent,
      linearTrackColor: c.tint,
      circularTrackColor: Colors.transparent,
    ),
    expansionTileTheme: ExpansionTileThemeData(
      iconColor: c.muted,
      collapsedIconColor: c.muted,
      textColor: c.text,
      collapsedTextColor: c.text,
      shape: const Border(),
      collapsedShape: const Border(),
    ),
    tooltipTheme: TooltipThemeData(
      waitDuration: const Duration(milliseconds: 600),
      decoration: BoxDecoration(
        color: c.surface,
        border: Border.all(color: c.border),
        borderRadius: BorderRadius.circular(ShepRadius.ghost),
      ),
      textStyle: style(ShepText.secondary),
    ),
    bottomSheetTheme: BottomSheetThemeData(
      backgroundColor: c.surface,
      surfaceTintColor: Colors.transparent,
    ),
    textSelectionTheme: TextSelectionThemeData(
      cursorColor: c.accent,
      selectionColor: c.tint,
      selectionHandleColor: c.accent,
    ),
  );
}

/// Desktop icon name for a swipe or context action.
String actionIconName(String name) => switch (name) {
  'archive' => 'archive',
  'trash' => 'trash',
  'read' => 'mail-open',
  'star' => 'flag',
  'select' => 'check-circle',
  'move' => 'move',
  'spam' => 'shield',
  _ => 'move',
};

Widget actionIcon(String name, {double? size, Color? color}) =>
    ShepIcon(actionIconName(name), size: size, color: color);
