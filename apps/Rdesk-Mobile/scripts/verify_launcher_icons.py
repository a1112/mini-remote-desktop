#!/usr/bin/env python3
"""Check source resources; optional merged-manifest and final-APK acceptance gates."""
import argparse
import json
import math
import re
import subprocess
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

from PIL import Image
from generate_launcher_icons import ROOT, RES, DENSITIES

ANDROID = "{http://schemas.android.com/apk/res/android}"


def check_manifest(path):
    root = ET.parse(path).getroot()
    app = root.find("application")
    assert app is not None, "Application missing"
    assert app.get(ANDROID + "icon") == "@mipmap/ic_launcher"
    assert app.get(ANDROID + "roundIcon") == "@mipmap/ic_launcher_round"
    launchers = []
    for element in list(app):
        for intent in element.findall("intent-filter"):
            actions = {e.get(ANDROID + "name") for e in intent.findall("action")}
            categories = {e.get(ANDROID + "name") for e in intent.findall("category")}
            if "android.intent.action.MAIN" in actions and "android.intent.category.LAUNCHER" in categories:
                launchers.append(element)
                for attr, expected in (("icon", "@mipmap/ic_launcher"), ("roundIcon", "@mipmap/ic_launcher_round")):
                    assert element.get(ANDROID + attr) in (None, expected), "Launcher overrides app icon"
    assert launchers, "MAIN/LAUNCHER entry missing"
    return [e.get(ANDROID + "name") for e in launchers]


def check_resources():
    for density, scale in DENSITIES.items():
        fg = Image.open(RES / f"drawable-{density}/ic_launcher_foreground.png").convert("RGBA")
        mono = Image.open(RES / f"drawable-{density}/ic_launcher_monochrome.png").convert("RGBA")
        assert fg.size == (round(108 * scale),) * 2 and mono.size == fg.size
        assert fg.getchannel("A").tobytes() == mono.getchannel("A").tobytes()
        assert all(r == g == b == 255 for r, g, b, a in mono.get_flattened_data())
        # Important artwork fits the 66dp safe circle. Tiny alpha<=8 export noise is recorded separately.
        cx = cy = (fg.width - 1) / 2
        radius = 0
        mask_clipped_pixels = 0
        for y in range(fg.height):
            for x in range(fg.width):
                a = fg.getpixel((x, y))[3]
                distance = math.hypot(x - cx, y - cy) / scale
                if a > 8:
                    radius = max(radius, distance)
                if a > 0 and distance > 36:
                    mask_clipped_pixels += 1
        assert radius <= 33, f"{density}: core artwork leaves 66dp safe circle ({radius})"
        assert mask_clipped_pixels == 0, f"{density}: 72dp circular viewport clips artwork"
        box = fg.getbbox()
        assert max(box[2] - box[0], box[3] - box[1]) / scale <= 66
        for name in ("ic_launcher", "ic_launcher_round"):
            legacy = Image.open(RES / f"mipmap-{density}/{name}.png")
            assert legacy.format == "PNG" and legacy.size == (round(48 * scale),) * 2
        print(f"PASS {density}: 108dp layers, 48dp legacy, core radius {radius:.2f}dp, no circular-mask clipping")
    for api in (26, 33):
        for name in ("ic_launcher", "ic_launcher_round"):
            xml = ET.parse(RES / f"mipmap-anydpi-v{api}/{name}.xml").getroot()
            assert xml.tag == "adaptive-icon"
            assert xml.find("background").get(ANDROID + "drawable") == "@color/ic_launcher_background"
            assert xml.find("foreground").get(ANDROID + "drawable") == "@drawable/ic_launcher_foreground"
            mono = xml.find("monochrome")
            assert (mono is not None) == (api == 33)
            if mono is not None:
                assert mono.get(ANDROID + "drawable") == "@drawable/ic_launcher_monochrome"


def check_apk(apk, aapt2, log_dir):
    log_dir.mkdir(parents=True, exist_ok=True)
    commands = {
        "badging": [aapt2, "dump", "badging", str(apk)],
        "manifest": [aapt2, "dump", "xmltree", str(apk), "--file", "AndroidManifest.xml"],
        "resources": [aapt2, "dump", "resources", str(apk)],
    }
    dumps = {}
    for name, command in commands.items():
        dumps[name] = subprocess.check_output(command, text=True, stderr=subprocess.STDOUT)
        (log_dir / f"aapt2-{name}.txt").write_text(dumps[name])
    resources, manifest = dumps["resources"], dumps["manifest"]
    for name, attr in (("ic_launcher", "icon"), ("ic_launcher_round", "roundIcon")):
        match = re.search(r"resource (0x[0-9a-fA-F]+) (?:\S+:)?mipmap/" + name + r"\b", resources)
        assert match, f"APK missing mipmap/{name}"
        assert re.search(r"android:" + attr + r"\([^\n]+=@" + match.group(1) + r"\b", manifest), f"Final manifest {attr} mismatch"
    assert "android.intent.action.MAIN" in manifest and "android.intent.category.LAUNCHER" in manifest
    assert "launchable-activity:" in dumps["badging"]
    assert "drawable/ic_launcher_foreground" in resources and "drawable/ic_launcher_monochrome" in resources
    with zipfile.ZipFile(apk) as archive:
        names = archive.namelist()
        for api in (26, 33):
            for name in ("ic_launcher", "ic_launcher_round"):
                candidates = [p for p in names if p.startswith(f"res/mipmap-anydpi-v{api}/") and p.endswith(f"/{name}.xml")]
                assert candidates, f"APK missing API {api} adaptive {name}"
                dump = subprocess.check_output([aapt2, "dump", "xmltree", str(apk), "--file", candidates[0]], text=True)
                (log_dir / f"aapt2-{name}-v{api}.txt").write_text(dump)
                assert "adaptive-icon" in dump and "foreground" in dump and "background" in dump
                assert ("monochrome" in dump) == (api == 33)
        for density in DENSITIES:
            # aapt may append its baseline API qualifier (e.g. -v4) to density directory names.
            for kind, stems in (("mipmap", ("ic_launcher", "ic_launcher_round")), ("drawable", ("ic_launcher_foreground", "ic_launcher_monochrome"))):
                for stem in stems:
                    assert any(re.match(rf"res/{kind}-{density}(?:-v\d+)?/{stem}\.png$", p) for p in names), f"APK missing {density} {stem}"
    print("PASS final APK: compiled manifest references, launcher, 5 densities, API26 adaptive and API33 monochrome")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--merged-manifest", type=Path)
    parser.add_argument("--apk", type=Path)
    parser.add_argument("--aapt2", default="aapt2")
    parser.add_argument("--log-dir", type=Path, default=ROOT / "build/launcher-verification")
    args = parser.parse_args()
    check_manifest(ROOT / "app/src/main/AndroidManifest.xml")
    check_resources()
    if args.merged_manifest:
        check_manifest(args.merged_manifest)
        print("PASS merged manifest: application icon/roundIcon and inherited launcher")
    else:
        print("NOT RUN: merged manifest (pass --merged-manifest after an Android build)")
    if args.apk:
        check_apk(args.apk, args.aapt2, args.log_dir)
    else:
        print("NOT RUN: final APK resources (pass --apk and --aapt2 after an Android build)")


if __name__ == "__main__":
    main()
