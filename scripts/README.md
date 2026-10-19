# Scripts

| Script | Purpose |
| --- | --- |
| `build-macos-app.sh` | Builds a release `olive-gui` and wraps it in `target/Olive Browser.app` (macOS only). CI runs it in the `package-macos` job. |
| `run-compatibility.sh` | Builds the `site-report` example and runs the opt-in live website audit. See *Website compatibility suite* in the main README. |
| `compatibility-report.py` | Audits each ranked domain in `tests/fixtures/compatibility/sites.tsv` in an isolated, time-bounded process and writes a JSON report. `run-compatibility.sh` calls it. |
| `render-app-icon.swift` | Draws the 1024×1024 app icon, `assets/olive-browser.png`, with Core Graphics. |
| `build-icns.py` | Packs a macOS `.iconset` folder into `assets/olive-browser.icns`. |

## Regenerating the app icon

Both icon files in `assets/` are generated. To change the icon, edit
`render-app-icon.swift`, then rebuild both files on macOS:

```sh
swift scripts/render-app-icon.swift assets/olive-browser.png
mkdir -p target/olive-browser.iconset
for spec in 16:icon_16x16 32:icon_32x32 128:icon_128x128 256:icon_128x128@2x \
    512:icon_256x256@2x 1024:icon_512x512@2x; do
  sips -z "${spec%%:*}" "${spec%%:*}" assets/olive-browser.png \
    --out "target/olive-browser.iconset/${spec#*:}.png"
done
python3 scripts/build-icns.py target/olive-browser.iconset assets/olive-browser.icns
```

Run unchanged, these commands reproduce the committed PNG and ICNS byte for byte.
The GUI embeds the PNG as its window icon, and the macOS app bundle uses the ICNS.
`assets/olive-browser.svg` is a vector drawing of the icon; no build step reads it.
