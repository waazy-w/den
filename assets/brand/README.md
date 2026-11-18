# den brand kit

| file | use |
| --- | --- |
| `den-icon-1024.png` / `-256` / `-64` | app icon (rounded warm-dark square) |
| `den.icns` | macOS icon, used for den's notifications |
| `den-mark.svg` / `den-mark.png` | the critter alone, transparent |
| `den-wordmark.svg` | critter + "den" in pixel letters (uses `currentColor`, so it follows the page's text color) |

Everything is drawn on the same pixel grid as the sprite in the terminal (16 × 10 square half-cells),
so keep edges crisp: scale by whole numbers and use nearest-neighbor resizing.

## Colors

| name | hex | where |
| --- | --- | --- |
| Den orange | `#D97757` | the critter, accents |
| Ink | `#1F1B18` | icon background, eyes, text on light |
| Cream | `#F5F1EA` | light backgrounds |

Regenerate everything with `python3 assets/brand/generate.py` (needs Pillow; `den.icns` needs macOS).
