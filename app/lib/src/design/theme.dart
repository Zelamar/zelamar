import 'package:flutter/material.dart';
import 'package:zelamar/src/design/colors.dart';
import 'package:zelamar/src/design/tokens.dart';
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

  final text = zelamarTextTheme(c.textPrimary, c.textSecondary);
  const pill = StadiumBorder();

  return ThemeData(
    brightness: brightness,
    colorScheme: scheme,
    scaffoldBackgroundColor: c.background,
    fontFamily: ZFonts.ui,
    textTheme: text,
    dividerTheme: DividerThemeData(color: c.divider, thickness: 1, space: 1),
    splashFactory: InkSparkle.splashFactory,
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        backgroundColor: c.accent,
        foregroundColor: c.onAccent,
        textStyle: text.labelLarge,
        shape: pill,
        minimumSize: const Size(0, 48),
        padding: const EdgeInsets.symmetric(horizontal: ZSpace.xl),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        foregroundColor: c.accent,
        side: BorderSide(color: c.hairline),
        textStyle: text.labelLarge,
        shape: pill,
        minimumSize: const Size(0, 44),
        padding: const EdgeInsets.symmetric(horizontal: ZSpace.lg),
      ),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: c.surface,
      hintStyle: text.bodyLarge?.copyWith(color: c.textTertiary),
      contentPadding: const EdgeInsets.symmetric(
        horizontal: ZSpace.lg,
        vertical: ZSpace.md,
      ),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(ZRadius.bubble),
        borderSide: BorderSide(color: c.hairline),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(ZRadius.bubble),
        borderSide: BorderSide(color: c.hairline),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(ZRadius.bubble),
        borderSide: BorderSide(color: c.accent, width: 1.5),
      ),
    ),
    tabBarTheme: TabBarThemeData(
      labelColor: c.textPrimary,
      unselectedLabelColor: c.textSecondary,
      labelStyle: text.titleSmall,
      unselectedLabelStyle: text.titleSmall,
      indicatorColor: c.accent,
      dividerColor: c.divider,
    ),
    snackBarTheme: SnackBarThemeData(
      backgroundColor: c.textPrimary,
      contentTextStyle: text.bodyMedium?.copyWith(color: c.background),
      behavior: SnackBarBehavior.floating,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(ZRadius.field),
      ),
    ),
    extensions: [c],
  );
}

/// Shorthand access to the design tokens: `context.colors.accent`.
extension ZelamarThemeContext on BuildContext {
  ZelamarColors get colors => Theme.of(this).extension<ZelamarColors>()!;
  TextTheme get text => Theme.of(this).textTheme;
}
