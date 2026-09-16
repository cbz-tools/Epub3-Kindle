# E-12 Embedded Font fixture

Purpose: exercise the AozoraEpub3 embedded-TTF path and the converter's KF8 FONT
resource handling as a focused generic EPUB 3 compatibility fixture.

## Placement

Place this directory at:

`tests/fixtures/embedded-font`

## Source assets

- `source.txt` — minimal AozoraEpub3 source text and a provenance/regeneration input.
- `css/vertical_font.css` — `@font-face` rule and a provenance/regeneration input.
- `fonts/EBGaramond12-Bold.ttf` — small OFL-1.1 TTF fixture font and a provenance/regeneration input.
- `licenses/EBGaramond-OFL-1.1.txt` — redistribution license/copyright notice and a provenance/regeneration input.

## Normal E-12 E2E

The normal E-12 integration test reads the checked-in `source.epub` directly. It
does not invoke AozoraEpub3 or mutate an external AozoraEpub3 checkout or its
templates at runtime. The `source.txt`, CSS, font, and license files above are
kept as provenance and regeneration inputs for the checked-in EPUB.

Font SHA-256:

`84E30A4F8A26CC1812C8072B01481A1E32CAB8A57B5F67BF413F59D60DC86B24`

## AozoraEpub3 staging intent

For fixture generation, use the repository's normal AozoraEpub3 command from
the sibling fixture-generation workflow:

1. stage `fonts/EBGaramond12-Bold.ttf` under AozoraEpub3
   `template/item/fonts/`;
2. stage the supplied CSS as `template/item/style_custom/font.css`;
3. add the fixture font's `application/font-sfnt` manifest item to the
   temporary `template/item/package.vm` used for generation;
4. generate the EPUB from `source.txt`;
5. restore all temporary mutations to the external AozoraEpub3 checkout.

AozoraEpub3 1.1.1b33Q copies the TTF and custom CSS but its stock package
template does not emit a manifest item for a general embedded font. The
checked-in `source.epub` was therefore produced through the same AozoraEpub3
workflow with only that temporary manifest-template addition; it was not
hand-edited as a ZIP.

The exact generation command/path should be taken from the repository's existing
fixture-generation workflow rather than guessed.

## E-12 assertions

The E2E should establish at minimum:

- the source EPUB contains the TTF manifest/resource;
- CSS contains an `@font-face` reference to that TTF;
- conversion succeeds;
- the generated KF8/AZW3 contains the corresponding FONT resource;
- the CSS/resource reference resolves to that FONT resource;
- the font resource bytes are structurally present and in-range;
- ordinary image-resource geometry remains valid.

Do not treat visual font rendering as the machine-verifiable E2E oracle.

## E-13

This fixture may provide evidence for the font-specific part of non-image
resource addressing, but it should not automatically claim all generic E-13
resource types unless those are independently exercised.
