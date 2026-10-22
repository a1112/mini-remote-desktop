# Rdesk Mobile launcher source

`launcher-source.png` is the selected B-family optical-v2 artwork for this
project (two cyan screens and an indigo lock). It is a real transparent RGBA
PNG, not SVG. This implementation uses the candidate artwork; independent
platform release acceptance and real-device acceptance remain pending.

Only this project's image and deterministic export metadata are included here.
The desktop Tauri ICO/ICNS/PNG files are unchanged by this Android fix, so this
change alone does not complete cross-platform branding.

`launcher-provenance.json` pins the source SHA-256, tool version, alpha crop,
resampling, layout parameters and every generated resource's SHA-256.
Generation does not repaint, trace, simplify or change the source artwork.
The monochrome layer preserves exactly the foreground alpha while providing
white RGB for system tinting. The new light background is platform-specific.

## Reproduce and check

From `apps/Rdesk-Mobile`, use Python 3.10+:

```sh
python3 -m pip install -r scripts/icon-requirements.txt
python3 scripts/generate_launcher_icons.py
python3 scripts/generate_launcher_icons.py --check
python3 scripts/test_launcher_checks.py
python3 scripts/verify_launcher_icons.py
```

The source has generous transparent margins. The exporter crops only those
margins and centers a 51dp alpha bounding box within each 108dp adaptive layer.
Important alpha (`>8/255`) is checked inside the 66dp safe circle; every
nonzero alpha pixel is checked against the 72dp circular viewport. A separate
48dp legacy export uses a 34dp symbol, with a circular variant for roundIcon.
Legacy resources are included even though the app currently requires API 29.
API 26 adaptive XML and API 33 monochrome XML share the same foreground.

With Java 17+, Gradle 8.13, installed/licensed Android platform 36 and
Build-Tools 35.0.0 (AGP 8.13.2 default):

```sh
ANDROID_HOME=/path/to/android-sdk bash scripts/build_and_verify_launcher.sh
```

The build script never installs SDK packages or accepts licenses. It runs unit
tests and assembles the real app, then checks the merged manifest and uses
`aapt2` on the final APK to verify the compiled icon resource references,
launcher, density resources, adaptive layers and monochrome layer. Logs are
written to `build/launcher-verification`. A static pass is not an APK pass.

For Windows, run the equivalent Gradle command in the main README, then call:

```powershell
python scripts/verify_launcher_icons.py --merged-manifest app/build/intermediates/merged_manifest/debug/processDebugMainManifest/AndroidManifest.xml --apk app/build/outputs/apk/debug/app-debug.apk --aapt2 "$env:ANDROID_HOME/build-tools/35.0.0/aapt2.exe"
```

Device checks still needed: install the built APK, inspect the launcher in the
actual OEM's supported shapes, and inspect themed icons on a supported launcher
with themed icons enabled. Refreshing an existing installed app may require a
launcher refresh; absence of a monochrome layer is not absence of a launcher.

Android reference: [Adaptive icons](https://developer.android.com/develop/ui/compose/system/icon_design_adaptive).
