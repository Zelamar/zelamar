# 0008. Design system lives in code

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

The CEO chose the **Ivory** direction and the **Sunset Z** emblem
(ROADMAP #28, #29) after rejecting a first round of dark, blue palettes. Design
feedback will now arrive continuously during development, so changing a color
or a size must be cheap, consistent everywhere, and must not silently break
readability.

## Decision

- **Tokens are code.** `app/lib/src/design/` is the single source of truth:
  - `colors.dart`: `ZelamarColors` theme extension with light (ivory and
    bronze) and warm dark (espresso, never black) palettes;
  - `typography.dart`: Cormorant Garamond for display, Geist for UI;
  - `tokens.dart`: spacing, radii, motion;
  - `theme.dart`: maps tokens onto Material.
  
  Widgets use `context.colors` and `context.text`, never raw hex values or
  font names.
- **Fonts are bundled** (SIL OFL 1.1, licenses in `app/assets/fonts/licenses/`)
  and never fetched at runtime: a request to a font CDN would leak usage
  metadata. They are variable fonts, so text styles set the `wght` axis
  explicitly.
- **Accessibility is a test.** `test/design_system_test.dart` asserts WCAG AA
  contrast (≥ 4.5:1) for every text/background pair in both themes. The first
  run caught two light-theme tokens at 4.47 and 4.39; they were darkened
  minimally.
- **The emblem is drawn, not imported.** `SunsetZPainter` paints it from a
  200×200 grid and switches to a simplified mark below 40 px.
  `scripts/generate_icons.py` (Pillow) renders the Android legacy and adaptive
  icons and the Windows `.ico` from the same geometry.

## Consequences

- A palette change is one edit plus a green contrast test.
- The emblem geometry exists twice (Dart painter, Python generator). Both cite
  the same grid; a change to one must be mirrored in the other and the icons
  regenerated.
- Python with Pillow becomes a developer prerequisite, but only for
  regenerating icons.
