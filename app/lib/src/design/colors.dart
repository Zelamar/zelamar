import 'package:flutter/material.dart';

/// Zelamar's "Ivory" palette: warm, quiet luxury.
///
/// Light is the primary theme; dark is warm espresso, never black. Blue,
/// cyan and black are deliberately absent (CEO decision, ROADMAP #28).
/// Widgets read colors only through `context.colors`, never hard-coded hex.
@immutable
class ZelamarColors extends ThemeExtension<ZelamarColors> {
  const new({
    required this.background,
    required this.surface,
    required this.surfaceRaised,
    required this.surfaceSunken,
    required this.hairline,
    required this.divider,
    required this.textPrimary,
    required this.textSecondary,
    required this.textTertiary,
    required this.accent,
    required this.onAccent,
    required this.bubbleOutgoing,
    required this.onBubbleOutgoing,
    required this.bubbleOutgoingMeta,
    required this.bubbleIncoming,
    required this.bubbleIncomingBorder,
    required this.online,
    required this.danger,
    required this.emblemTile,
    required this.emblemMark,
  });

  /// Ivory on bronze: the primary theme.
  static const light = ZelamarColors(
    background: Color(0xFFF8F4EE),
    surface: Color(0xFFFFFFFF),
    surfaceRaised: Color(0xFFFBF8F3),
    surfaceSunken: Color(0xFFEFE8DD),
    hairline: Color(0xFFE8DFD2),
    divider: Color(0xFFEAE2D6),
    textPrimary: Color(0xFF2B231D),
    textSecondary: Color(0xFF6F645A),
    textTertiary: Color(0xFF776C61),
    accent: Color(0xFF8A5A30),
    onAccent: Color(0xFFFFFFFF),
    bubbleOutgoing: Color(0xFFEADFCD),
    onBubbleOutgoing: Color(0xFF2B231D),
    bubbleOutgoingMeta: Color(0xFF6A5E52),
    bubbleIncoming: Color(0xFFFFFFFF),
    bubbleIncomingBorder: Color(0xFFECE4D8),
    online: Color(0xFF4F6A42),
    danger: Color(0xFFA23B2C),
    emblemTile: Color(0xFFF8F3EA),
    emblemMark: Color(0xFF8A5A30),
  );

  /// Warm espresso: the dark theme.
  static const dark = ZelamarColors(
    background: Color(0xFF221C17),
    surface: Color(0xFF2B231D),
    surfaceRaised: Color(0xFF2F2620),
    surfaceSunken: Color(0xFF3A3029),
    hairline: Color(0xFF3F352D),
    divider: Color(0xFF362D26),
    textPrimary: Color(0xFFF3EDE4),
    textSecondary: Color(0xFFBFB3A6),
    textTertiary: Color(0xFFA69A8D),
    accent: Color(0xFFD2A878),
    onAccent: Color(0xFF2B231D),
    bubbleOutgoing: Color(0xFF4A3A2C),
    onBubbleOutgoing: Color(0xFFF3EDE4),
    bubbleOutgoingMeta: Color(0xFFCDBFAE),
    bubbleIncoming: Color(0xFF2F2620),
    bubbleIncomingBorder: Color(0xFF3F352D),
    online: Color(0xFF8FAE7E),
    danger: Color(0xFFE08A78),
    emblemTile: Color(0xFF2B231D),
    emblemMark: Color(0xFFD9C3A0),
  );

  /// Page background.
  final Color background;

  /// Cards, incoming bubbles, input fields.
  final Color surface;

  /// Bars that sit above the page: headers, composer.
  final Color surfaceRaised;

  /// Recessed controls: search field, secondary buttons, placeholders.
  final Color surfaceSunken;

  /// 1px borders of raised elements.
  final Color hairline;

  /// List separators.
  final Color divider;

  /// Body text and titles.
  final Color textPrimary;

  /// Previews, captions, secondary labels.
  final Color textSecondary;

  /// Timestamps and other quiet metadata.
  final Color textTertiary;

  /// Bronze: primary actions, unread badges, read receipts.
  final Color accent;

  /// Text and icons drawn on [accent].
  final Color onAccent;

  /// Messages sent by the user (tone-on-tone sand).
  final Color bubbleOutgoing;

  /// Text on [bubbleOutgoing].
  final Color onBubbleOutgoing;

  /// Timestamps inside [bubbleOutgoing].
  final Color bubbleOutgoingMeta;

  /// Messages received from others.
  final Color bubbleIncoming;

  /// 1px border of [bubbleIncoming].
  final Color bubbleIncomingBorder;

  /// Presence dot and "online" label.
  final Color online;

  /// Errors and destructive actions.
  final Color danger;

  /// Background of the Sunset Z emblem tile.
  final Color emblemTile;

  /// The Sunset Z mark itself.
  final Color emblemMark;

  @override
  ZelamarColors copyWith({
    Color? background,
    Color? surface,
    Color? surfaceRaised,
    Color? surfaceSunken,
    Color? hairline,
    Color? divider,
    Color? textPrimary,
    Color? textSecondary,
    Color? textTertiary,
    Color? accent,
    Color? onAccent,
    Color? bubbleOutgoing,
    Color? onBubbleOutgoing,
    Color? bubbleOutgoingMeta,
    Color? bubbleIncoming,
    Color? bubbleIncomingBorder,
    Color? online,
    Color? danger,
    Color? emblemTile,
    Color? emblemMark,
  }) {
    return ZelamarColors(
      background: background ?? this.background,
      surface: surface ?? this.surface,
      surfaceRaised: surfaceRaised ?? this.surfaceRaised,
      surfaceSunken: surfaceSunken ?? this.surfaceSunken,
      hairline: hairline ?? this.hairline,
      divider: divider ?? this.divider,
      textPrimary: textPrimary ?? this.textPrimary,
      textSecondary: textSecondary ?? this.textSecondary,
      textTertiary: textTertiary ?? this.textTertiary,
      accent: accent ?? this.accent,
      onAccent: onAccent ?? this.onAccent,
      bubbleOutgoing: bubbleOutgoing ?? this.bubbleOutgoing,
      onBubbleOutgoing: onBubbleOutgoing ?? this.onBubbleOutgoing,
      bubbleOutgoingMeta: bubbleOutgoingMeta ?? this.bubbleOutgoingMeta,
      bubbleIncoming: bubbleIncoming ?? this.bubbleIncoming,
      bubbleIncomingBorder: bubbleIncomingBorder ?? this.bubbleIncomingBorder,
      online: online ?? this.online,
      danger: danger ?? this.danger,
      emblemTile: emblemTile ?? this.emblemTile,
      emblemMark: emblemMark ?? this.emblemMark,
    );
  }

  @override
  ZelamarColors lerp(ZelamarColors? other, double t) {
    if (other == null) return this;
    Color mix(Color a, Color b) => Color.lerp(a, b, t)!;
    return ZelamarColors(
      background: mix(background, other.background),
      surface: mix(surface, other.surface),
      surfaceRaised: mix(surfaceRaised, other.surfaceRaised),
      surfaceSunken: mix(surfaceSunken, other.surfaceSunken),
      hairline: mix(hairline, other.hairline),
      divider: mix(divider, other.divider),
      textPrimary: mix(textPrimary, other.textPrimary),
      textSecondary: mix(textSecondary, other.textSecondary),
      textTertiary: mix(textTertiary, other.textTertiary),
      accent: mix(accent, other.accent),
      onAccent: mix(onAccent, other.onAccent),
      bubbleOutgoing: mix(bubbleOutgoing, other.bubbleOutgoing),
      onBubbleOutgoing: mix(onBubbleOutgoing, other.onBubbleOutgoing),
      bubbleOutgoingMeta: mix(bubbleOutgoingMeta, other.bubbleOutgoingMeta),
      bubbleIncoming: mix(bubbleIncoming, other.bubbleIncoming),
      bubbleIncomingBorder: mix(
        bubbleIncomingBorder,
        other.bubbleIncomingBorder,
      ),
      online: mix(online, other.online),
      danger: mix(danger, other.danger),
      emblemTile: mix(emblemTile, other.emblemTile),
      emblemMark: mix(emblemMark, other.emblemMark),
    );
  }
}
