import 'package:flutter/material.dart';

/// Font families bundled with the app (never fetched at runtime: a request
/// to a font CDN would leak usage metadata). Licenses: SIL OFL 1.1, see
/// `assets/fonts/licenses/`.
abstract final class ZFonts {
  /// Display serif: screen titles, names in headers, initials, wordmark.
  static const String display = 'CormorantGaramond';

  /// UI sans: everything else.
  static const String ui = 'Geist';
}

/// Builds a text style for one of the bundled variable fonts.
///
/// Variable fonts need the `wght` axis set explicitly; [FontWeight] alone
/// would make Flutter synthesize a fake bold.
TextStyle _style(
  String family,
  double size,
  int weight, {
  double? height,
  double letterSpacing = 0,
}) {
  return TextStyle(
    fontFamily: family,
    fontSize: size,
    fontWeight: FontWeight.values[(weight ~/ 100) - 1],
    fontVariations: [FontVariation.weight(weight.toDouble())],
    height: height,
    letterSpacing: letterSpacing,
  );
}

/// Zelamar's type scale, mapped onto Material's [TextTheme] slots so that
/// standard widgets pick it up automatically.
TextTheme zelamarTextTheme(Color primary, Color secondary) {
  final base = TextTheme(
    // "Chats" and other screen titles.
    displayMedium: _style(
      ZFonts.display,
      42,
      600,
      height: 1.05,
      letterSpacing: 0.2,
    ),
    // The wordmark on splash and about screens.
    displaySmall: _style(
      ZFonts.display,
      36,
      500,
      height: 1.1,
      letterSpacing: 1.8,
    ),
    // Contact name in a conversation header.
    headlineSmall: _style(ZFonts.display, 22, 600, height: 1.1),
    // Section titles.
    titleLarge: _style(ZFonts.ui, 18, 600, height: 1.3),
    // Names in lists.
    titleMedium: _style(ZFonts.ui, 15.5, 600, height: 1.3),
    titleSmall: _style(ZFonts.ui, 14, 600, height: 1.3),
    // Message text.
    bodyLarge: _style(ZFonts.ui, 15, 400, height: 1.42),
    // Previews and secondary text.
    bodyMedium: _style(ZFonts.ui, 14, 400, height: 1.4),
    bodySmall: _style(ZFonts.ui, 12.5, 400, height: 1.35),
    // Buttons.
    labelLarge: _style(ZFonts.ui, 15, 600, height: 1.2),
    labelMedium: _style(ZFonts.ui, 12, 600, height: 1.2, letterSpacing: 1.6),
    // Timestamps.
    labelSmall: _style(ZFonts.ui, 11, 400, height: 1.2),
  );
  return base
      .apply(bodyColor: primary, displayColor: primary)
      .copyWith(
        bodyMedium: base.bodyMedium?.copyWith(color: secondary),
        bodySmall: base.bodySmall?.copyWith(color: secondary),
        labelSmall: base.labelSmall?.copyWith(color: secondary),
      );
}
