import 'dart:convert';
import 'package:flutter/material.dart';
import 'mail.dart';

class Preferences {
  const Preferences({
    this.appearance = ThemeMode.system,
    this.leftSwipe = MailAction.archive,
    this.rightSwipe = MailAction.read,
    this.previewLines = 2,
    this.avatars = true,
    this.unified = true,
    this.quoteMode = 'Collapsed',
    this.tooltips = true,
  });
  final ThemeMode appearance;
  final MailAction leftSwipe, rightSwipe;
  final int previewLines;
  final bool avatars, unified, tooltips;
  final String quoteMode;

  Preferences copy({
    ThemeMode? appearance,
    MailAction? leftSwipe,
    MailAction? rightSwipe,
    int? previewLines,
    bool? avatars,
    bool? unified,
    String? quoteMode,
    bool? tooltips,
  }) => Preferences(
    appearance: appearance ?? this.appearance,
    leftSwipe: leftSwipe ?? this.leftSwipe,
    rightSwipe: rightSwipe ?? this.rightSwipe,
    previewLines: previewLines ?? this.previewLines,
    avatars: avatars ?? this.avatars,
    unified: unified ?? this.unified,
    quoteMode: quoteMode ?? this.quoteMode,
    tooltips: tooltips ?? this.tooltips,
  );

  String encode() => jsonEncode({
    'version': 1,
    'appearance': appearance.name,
    'leftSwipe': leftSwipe.name,
    'rightSwipe': rightSwipe.name,
    'previewLines': previewLines,
    'avatars': avatars,
    'unified': unified,
    'quoteMode': quoteMode,
    'tooltips': tooltips,
  });

  static Preferences decode(String? raw) {
    if (raw == null) return const Preferences();
    final data = jsonDecode(raw) as Map<String, dynamic>;
    if (data['version'] != 1) {
      throw const FormatException('Unknown preferences version');
    }
    T pick<T extends Enum>(List<T> values, String key, T fallback) =>
        values.where((v) => v.name == data[key]).firstOrNull ?? fallback;
    return Preferences(
      appearance: pick(ThemeMode.values, 'appearance', ThemeMode.system),
      leftSwipe: pick(MailAction.values, 'leftSwipe', MailAction.archive),
      rightSwipe: pick(MailAction.values, 'rightSwipe', MailAction.read),
      previewLines: ((data['previewLines'] as int?) ?? 2).clamp(0, 4),
      avatars: data['avatars'] as bool? ?? true,
      unified: data['unified'] as bool? ?? true,
      quoteMode:
          ['Collapsed', 'Expanded', 'Latest only'].contains(data['quoteMode'])
          ? data['quoteMode'] as String
          : 'Collapsed',
      tooltips: data['tooltips'] as bool? ?? true,
    );
  }
}
