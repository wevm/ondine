---
'ondine': patch
---

Added the `theme` render option, which writes the theme into the SVG as literal values. The output has no `var()` or `color-mix()`, so renderers without CSS custom properties draw it, such as resvg, librsvg, and native SVG parsers.
