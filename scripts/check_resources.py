#!/usr/bin/env python3
"""Check fallback strings, duplicate Fluent keys, and desktop asset identity."""
from pathlib import Path
import re
import xml.etree.ElementTree as ET

root = Path(__file__).resolve().parents[1]
errors = []
resources = {}
for path in sorted((root / "i18n").glob("*/*.ftl")):
    keys = re.findall(r"^([\w-]+)\s*=", path.read_text(), re.M)
    duplicates = sorted({key for key in keys if keys.count(key) > 1})
    if duplicates:
        errors.append(f"{path.relative_to(root)}: duplicate keys: {duplicates}")
    resources[path.parent.name] = set(keys)
fallback = resources["en"]
for path in (root / "src").rglob("*.rs"):
    for key in re.findall(r'\bfl!\(\s*"([\w-]+)"', path.read_text()):
        if key not in fallback:
            errors.append(f"{path.relative_to(root)}: missing English fallback: {key}")
appid = "dev.edfloreshz.Toot"
metadata = ET.parse(root / f"res/{appid}.metainfo.xml").getroot()
assert metadata.findtext("id") == appid
assert metadata.findtext("launchable") == f"{appid}.desktop"
assert f"Icon={appid}\n" in (root / "res/app.desktop").read_text()
icon = ET.parse(root / "res/icons/hicolor/scalable/apps/icon.svg").getroot()
assert len(icon) > 0, "App icon is empty"
if errors:
    raise SystemExit("\n".join(errors))
for language, keys in resources.items():
    print(f"{language}: {len(keys & fallback)}/{len(fallback)} strings; missing strings use English")
print("Resource checks passed.")
