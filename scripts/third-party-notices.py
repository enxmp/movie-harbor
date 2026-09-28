"""Collect package-supplied license texts after npm ci and cargo fetch."""
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--locked", "--manifest-path", str(root / "src-tauri/Cargo.toml"),
    "--format-version", "1",
]))
packages = []
for package in metadata["packages"]:
    if package["source"]:
        packages.append((package["name"], package["version"], package.get("license"),
                         Path(package["manifest_path"]).parent))
lock = json.loads((root / "package-lock.json").read_text(encoding="utf-8"))
for location, item in lock["packages"].items():
    folder = root / location
    if location and (folder / "package.json").is_file():
        package = json.loads((folder / "package.json").read_text(encoding="utf-8"))
        packages.append((package["name"], package["version"], package.get("license"), folder))

output = ["Movie Harbor — third-party license texts\n",
          "Includes build-time and platform-specific packages; not all are part of each binary.\n",
          "FFmpeg is installed separately and is not distributed in these releases.\n"]
missing = []
for name, version, license_name, folder in sorted(packages, key=lambda p: (p[0], p[1])):
    files = [p for p in folder.iterdir() if p.is_file() and
             p.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE"))]
    for subdir in ("LICENSES", "licenses"):
        if (folder / subdir).is_dir():
            files += [p for p in (folder / subdir).rglob("*") if p.is_file()]
    supplemental = root / "docs/licenses" / f"{name}-{version}.txt"
    if not files and supplemental.is_file():
        files = [supplemental]
    output.append(f"\n{'=' * 72}\n{name} {version}\nDeclared license: {license_name}\n")
    if not files:
        missing.append(f"{name} {version}")
        output.append("See the published package for its license and attribution.\n")
    for file in sorted(set(files)):
        output.append(f"\n--- {file.name} ---\n")
        output.append(file.read_text(encoding="utf-8", errors="replace") + "\n")
(root / "THIRD_PARTY_LICENSES.txt").write_text("".join(output), encoding="utf-8")
print(f"Collected {len(packages)} package notices. Packages without a license file: {len(missing)}")
for item in missing:
    print(item)
