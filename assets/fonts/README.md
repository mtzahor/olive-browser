# Bundled fonts

## Inter

`InterVariable.ttf` is the Inter variable font by Rasmus Andersson, distributed
under the SIL Open Font License in `OFL.txt`. Downloaded from the
[official Inter repository](https://github.com/rsms/inter/tree/master/docs/font-files)
on 2026-09-06. It provides readable UI/document text and real bold weights.
The font is embedded only in the optional GUI executable.

## Noto Sans Hebrew

`NotoSansHebrew.ttf` is the unmodified Noto Sans Hebrew variable font from the
[Google Fonts repository](https://github.com/google/fonts/tree/main/ofl/notosanshebrew),
downloaded on 2026-09-08. Its upstream filename is `NotoSansHebrew[wdth,wght].ttf`.
It is distributed under the SIL Open Font License in `NotoSansHebrew-OFL.txt`.

- Git blob SHA-1: `f31f73b8be8086692ff0156544f80975640e5a42`
- SHA-256: `7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88`

The GUI embeds this face as a fallback for proportional and monospace text,
including page content and browser controls. It supplies Hebrew letters and
vowel marks missing from Inter and egui's default fonts, and supports regular
and bold weights. Fallback requires neither installed system fonts nor downloads
from a website. Adding glyph coverage does not implement full bidirectional layout.
