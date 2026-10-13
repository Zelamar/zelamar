import 'package:flutter/animation.dart';

/// Spacing scale. Layouts use these names, never raw numbers.
abstract final class ZSpace {
  static const double xxs = 2;
  static const double xs = 4;
  static const double sm = 8;
  static const double md = 12;
  static const double lg = 16;

  /// Horizontal page gutter.
  static const double gutter = 22;
  static const double xl = 24;
  static const double xxl = 32;
  static const double xxxl = 48;
}

/// Corner radii.
abstract final class ZRadius {
  /// Message bubbles.
  static const double bubble = 20;

  /// The corner of a bubble that points at its sender.
  static const double bubbleTail = 6;

  /// Search fields, chips, small cards.
  static const double field = 12;

  /// Large cards and sheets.
  static const double card = 22;

  /// App-icon style tiles (emblem): 46/200 of the tile size.
  static const double tileRatio = 0.23;
}

/// Motion: calm and precise. Nothing bounces.
abstract final class ZMotion {
  static const Duration fast = Duration(milliseconds: 120);
  static const Duration base = Duration(milliseconds: 200);
  static const Duration slow = Duration(milliseconds: 320);

  /// Elements entering the screen.
  static const Curve enter = Curves.easeOutCubic;

  /// Elements leaving the screen.
  static const Curve exit = Curves.easeInCubic;

  /// Shared-axis transitions between screens.
  static const Curve emphasized = Cubic(0.2, 0, 0, 1);
}
