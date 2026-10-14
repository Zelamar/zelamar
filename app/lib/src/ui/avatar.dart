import 'package:flutter/material.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/typography.dart';

/// A person's monogram: the first letter of their name, set in the display
/// serif on a quiet sand disc. No photos until avatars are end-to-end
/// encrypted too.
class Avatar extends StatelessWidget {
  const new(this.name, {this.size = 48, super.key});

  /// The name to draw from, such as `@bob`.
  final String name;
  final double size;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final letter = name
        .replaceAll('@', '')
        .trim()
        .characters
        .firstOrNull
        ?.toUpperCase();
    return Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: colors.surfaceSunken,
        shape: BoxShape.circle,
        border: Border.all(color: colors.hairline),
      ),
      child: Text(
        letter ?? '·',
        style: TextStyle(
          fontFamily: ZFonts.display,
          fontSize: size * 0.46,
          fontWeight: FontWeight.w600,
          fontVariations: const [FontVariation.weight(600)],
          color: colors.accent,
          height: 1,
        ),
      ),
    );
  }
}
