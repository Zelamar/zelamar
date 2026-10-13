import 'package:flutter/widgets.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';

/// The Zelamar emblem, "Sunset Z": a sun setting behind the horizon whose
/// reflection on the water steps down into the letter Z.
///
/// Drawn in code rather than shipped as an image, so it stays razor sharp
/// at every size and follows the theme. Below 40 logical pixels it switches
/// to a simplified mark with thicker strokes, the way real app icons do.
class SunsetZEmblem extends StatelessWidget {
  const new({
    this.size = 120,
    this.tileColor,
    this.markColor,
    this.showTile = true,
    super.key,
  });

  /// Edge length of the square emblem.
  final double size;

  /// Tile background; defaults to the theme's `emblemTile`.
  final Color? tileColor;

  /// Mark color; defaults to the theme's `emblemMark`.
  final Color? markColor;

  /// Whether to paint the rounded tile behind the mark.
  final bool showTile;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Semantics(
      label: 'Zelamar',
      image: true,
      child: CustomPaint(
        size: Size.square(size),
        painter: SunsetZPainter(
          tileColor: showTile ? (tileColor ?? colors.emblemTile) : null,
          markColor: markColor ?? colors.emblemMark,
        ),
      ),
    );
  }
}

/// Paints the Sunset Z mark on a 200x200 design grid scaled to the canvas.
class SunsetZPainter extends CustomPainter {
  const new({required this.markColor, this.tileColor});

  /// Rounded tile behind the mark; `null` paints the mark alone.
  final Color? tileColor;

  /// Color of the sun, the horizon and the reflection.
  final Color markColor;

  static const double _grid = 200;

  @override
  void paint(Canvas canvas, Size size) {
    final scale = size.shortestSide / _grid;
    canvas
      ..save()
      ..scale(scale);

    final tile = tileColor;
    if (tile != null) {
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(0, 0, _grid, _grid),
          const Radius.circular(_grid * ZRadius.tileRatio),
        ),
        Paint()..color = tile,
      );
    }

    final compact = size.shortestSide < 40;
    final fill = Paint()
      ..color = markColor
      ..isAntiAlias = true;
    final stroke = Paint()
      ..color = markColor
      ..style = PaintingStyle.stroke
      ..strokeCap = StrokeCap.round
      ..strokeWidth = compact ? 14 : (size.shortestSide < 80 ? 9 : 5)
      ..isAntiAlias = true;

    // The sun: a half disc resting just above the horizon.
    final sunRadius = compact ? 36.0 : 30.0;
    canvas.drawArc(
      Rect.fromCircle(center: const Offset(100, 84), radius: sunRadius),
      3.141592653589793,
      3.141592653589793,
      true,
      fill,
    );

    // Horizon (top bar of the Z), reflection steps (its diagonal) and the
    // far shore (bottom bar).
    final lines = compact ? _compactLines : _fullLines;
    for (final (start, end) in lines) {
      canvas.drawLine(start, end, stroke);
    }

    canvas.restore();
  }

  static const _fullLines = [
    (Offset(48, 94), Offset(152, 94)),
    (Offset(106, 108), Offset(140, 108)),
    (Offset(83, 122), Offset(117, 122)),
    (Offset(60, 136), Offset(94, 136)),
    (Offset(48, 150), Offset(152, 150)),
  ];

  static const _compactLines = [
    (Offset(46, 100), Offset(154, 100)),
    (Offset(78, 130), Offset(122, 130)),
    (Offset(46, 160), Offset(154, 160)),
  ];

  @override
  bool shouldRepaint(SunsetZPainter oldDelegate) =>
      oldDelegate.tileColor != tileColor || oldDelegate.markColor != markColor;
}
