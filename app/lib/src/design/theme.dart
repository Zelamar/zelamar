import 'package:flutter/material.dart';
import 'package:zelamar/src/design/colors.dart';
import 'package:zelamar/src/design/typography.dart';

/// Builds Zelamar's [ThemeData] for one brightness from the design tokens.
ThemeData zelamarTheme(Brightness brightness) {
  final c = brightness == Brightness.light
      ? ZelamarColors.light
      : ZelamarColors.dark;

  final scheme = ColorScheme(
    brightness: brightness,
    primary: c.accent,
    onPrimary: c.onAccent,
    secondary: c.accent,
    onSecondary: c.onAccent,
    error: c.danger,
    onError: c.onAccent,
    surface: c.background,
    onSurface: c.textPrimary,
    onSurfaceVariant: c.textSecondary,
    surfaceContainerLowest: c.surface,
    surfaceContainerLow: c.surfaceRaised,
    surfaceContainer: c.surfaceRaised,
    surfaceContainerHigh: c.surfaceSunken,
    surfaceContainerHighest: c.surfaceSunken,
    outline: c.hairline,
    outlineVariant: c.divider,
  );

  return ThemeData(
    brightness: brightness,
    colorScheme: scheme,
    scaffoldBackgroundColor: c.background,
    fontFamily: ZFonts.ui,
    textTheme: zelamarTextTheme(c.textPrimary, c.textSecondary),
    dividerTheme: DividerThemeData(color: c.divider, thickness: 1, space: 1),
    splashFactory: InkSparkle.splashFactory,
    extensions: [c],
  );
}

/// Shorthand access to the design tokens: `context.colors.accent`.
extension ZelamarThemeContext on BuildContext {
  ZelamarColors get colors => Theme.of(this).extension<ZelamarColors>()!;
  TextTheme get text => Theme.of(this).textTheme;
}
