import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/design/colors.dart';
import 'package:zelamar/src/design/emblem.dart';
import 'package:zelamar/src/design/theme.dart';

/// WCAG 2.x contrast ratio between two opaque colors.
double contrast(Color a, Color b) {
  final la = a.computeLuminance();
  final lb = b.computeLuminance();
  return (math.max(la, lb) + 0.05) / (math.min(la, lb) + 0.05);
}

void main() {
  group('Ivory palette meets WCAG AA', () {
    // Normal-size text needs 4.5:1. Every pair below carries text.
    const aa = 4.5;

    for (final (name, c) in [
      ('light', ZelamarColors.light),
      ('dark', ZelamarColors.dark),
    ]) {
      final pairs = <String, (Color, Color)>{
        'text on background': (c.textPrimary, c.background),
        'secondary text on background': (c.textSecondary, c.background),
        'tertiary text on background': (c.textTertiary, c.background),
        'secondary text on sunken surface': (c.textSecondary, c.surfaceSunken),
        'text on accent': (c.onAccent, c.accent),
        'accent text on background': (c.accent, c.background),
        'text in outgoing bubble': (c.onBubbleOutgoing, c.bubbleOutgoing),
        'timestamp in outgoing bubble': (
          c.bubbleOutgoingMeta,
          c.bubbleOutgoing,
        ),
        'text in incoming bubble': (c.textPrimary, c.bubbleIncoming),
        'timestamp in incoming bubble': (c.textTertiary, c.bubbleIncoming),
        'online label on background': (c.online, c.background),
        'error text on background': (c.danger, c.background),
      };

      for (final MapEntry(key: label, value: (fg, bg)) in pairs.entries) {
        test('$name: $label', () {
          expect(contrast(fg, bg), greaterThanOrEqualTo(aa));
        });
      }
    }
  });

  group('theme', () {
    test('both brightnesses expose the Zelamar tokens', () {
      for (final brightness in Brightness.values) {
        final theme = zelamarTheme(brightness);
        expect(theme.brightness, brightness);
        expect(theme.extension<ZelamarColors>(), isNotNull);
      }
    });

    test('light is ivory and bronze, dark is warm espresso', () {
      final light = zelamarTheme(Brightness.light);
      final dark = zelamarTheme(Brightness.dark);
      expect(light.scaffoldBackgroundColor, ZelamarColors.light.background);
      expect(light.colorScheme.primary, ZelamarColors.light.accent);
      expect(dark.scaffoldBackgroundColor, ZelamarColors.dark.background);
      // Warm, never black: red channel dominates blue in the dark ground.
      final ground = ZelamarColors.dark.background;
      expect(ground.r, greaterThan(ground.b));
    });

    test('lerp interpolates every token', () {
      final mid = ZelamarColors.light.lerp(ZelamarColors.dark, 0.5);
      expect(mid.accent, isNot(ZelamarColors.light.accent));
      expect(mid.accent, isNot(ZelamarColors.dark.accent));
    });
  });

  group('Sunset Z emblem', () {
    Widget host(Widget child) => MaterialApp(
      theme: zelamarTheme(Brightness.light),
      home: Center(child: child),
    );

    testWidgets('renders at full and compact sizes', (tester) async {
      for (final size in [120.0, 28.0]) {
        await tester.pumpWidget(host(SunsetZEmblem(size: size)));
        final paint = tester.widget<CustomPaint>(
          find.descendant(
            of: find.byType(SunsetZEmblem),
            matching: find.byType(CustomPaint),
          ),
        );
        expect(paint.size, Size.square(size));
        expect(paint.painter, isA<SunsetZPainter>());
      }
    });

    testWidgets('is announced as "Zelamar" to screen readers', (tester) async {
      await tester.pumpWidget(host(const SunsetZEmblem()));
      expect(find.bySemanticsLabel('Zelamar'), findsOneWidget);
    });

    test('repaints only when colors change', () {
      const a = SunsetZPainter(markColor: Color(0xFF000001));
      const b = SunsetZPainter(markColor: Color(0xFF000001));
      const c = SunsetZPainter(markColor: Color(0xFF000002));
      expect(a.shouldRepaint(b), isFalse);
      expect(a.shouldRepaint(c), isTrue);
    });
  });
}
