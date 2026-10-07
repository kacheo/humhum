"""Generate redistributable notices from cargo-about JSON and installed npm packages.

First generate cargo-about JSON with the checked-in about.toml, then pass its
path as the sole argument. Run after npm ci, from the repository root.
"""
import json
import hashlib
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
report = json.loads(Path(sys.argv[1]).read_text())
output = ROOT / "docs/licenses"
output.mkdir(parents=True, exist_ok=True)
overrides = {}
for item in json.loads((output / "upstream/manifest.json").read_text()):
    for component in item["components"]:
        overrides[component] = item
rust = ["Rust production and build dependencies\nGenerated with cargo-about 0.9.2; target aarch64-apple-darwin; dev dependencies excluded.\n"]
inventory = ["# Dependency inventory\n", "Production/build dependencies for Apple Silicon. Build-only crates are conservatively included. Development tooling and other platforms are excluded.\n", "## Rust\n", "| Component | Version | Declared license | Source |", "| --- | --- | --- | --- |"]
for crate in sorted(report["crates"], key=lambda c: (c["package"]["name"], c["package"]["version"])):
    p = crate["package"]
    source = f"https://crates.io/api/v1/crates/{p['name']}/{p['version']}/download" if p["source"] else f"https://github.com/kacheo/wipzi/tree/v{p['version']}"
    inventory.append(f"| {p['name']} | {p['version']} | {p['license']} | [Source]({source}) |")
for license in report["licenses"]:
    users = sorted({f"{u['crate']['name']} {u['crate']['version']}" for u in license["used_by"]})
    if "Copyright (c) <year>" in license["text"]:
        # cargo-about synthesizes placeholders when crates omit license files.
        # Use reviewed exact-source overrides or actual packaged notices.
        for user in license["used_by"]:
            p = user["crate"]
            component = f"{p['name']}@{p['version']}"
            item = overrides.get(component)
            if item:
                text = (output / "upstream" / item["file"]).read_text()
                if item.get("standard_mit"):
                    text += "\n\nStandard MIT grant (upstream attribution above):\n" + license["text"].replace("Copyright (c) <year> <copyright holders>\n", "")
                text += "\n\nAttribution source: " + item["source"]
            else:
                base = Path(p["manifest_path"]).parent
                notices = [f for f in sorted(base.iterdir()) if f.is_file() and f.name.upper().startswith(("LICENSE", "NOTICE", "COPYRIGHT"))]
                texts = [f.read_text() for f in notices if "Permission is hereby granted" in f.read_text() and "Copyright (c) <year>" not in f.read_text()]
                if not texts:
                    raise RuntimeError(f"Unresolved attribution for {component}; review upstream notices first")
                text = "\n\n".join(texts)
            rust.extend(["\n" + "=" * 72, component, text.strip()])
    else:
        rust.extend([f"\n{'=' * 72}\n{license['name']} ({license['id']})", "Components: " + ", ".join(users), license["text"].strip()])
# whisper-rs-sys vendors whisper.cpp/ggml under a separate MIT license.
sys_crate = next(c["package"] for c in report["crates"] if c["package"]["name"] == "whisper-rs-sys")
native_license = Path(sys_crate["manifest_path"]).parent / "whisper.cpp/LICENSE"
rust.extend(["\n" + "=" * 72, "Vendored whisper.cpp / ggml in whisper-rs-sys", native_license.read_text().strip()])
ring = next((c["package"] for c in report["crates"] if c["package"]["name"] == "ring"), None)
if ring:
    for name in ("LICENSE", "LICENSE-other-bits", "LICENSE-BoringSSL"):
        path = Path(ring["manifest_path"]).parent / name
        rust.extend(["\n" + "=" * 72, f"ring native components: {name}", path.read_text().strip()])
(output / "RUST_NOTICES.txt").write_text("\n\n".join(rust) + "\n")

paths = subprocess.check_output(["npm", "ls", "--omit=dev", "--all", "--parseable"], cwd=ROOT, text=True).splitlines()
npm = ["npm production dependencies\nGenerated from installed package versions after npm ci; development dependencies excluded.\n"]
inventory.extend(["\n## npm\n", "| Component | Version | Declared license |", "| --- | --- | --- |"])
for path in sorted(paths[1:]):
    base = Path(path)
    p = json.loads((base / "package.json").read_text())
    texts = sorted(f for f in base.iterdir() if f.is_file() and f.name.upper().startswith(("LICENSE", "LICENCE", "NOTICE", "COPYRIGHT")))
    if not texts:
        raise RuntimeError(f"Missing packaged license text for {p['name']}; retrieve and review upstream notices before distributing")
    inventory.append(f"| {p['name']} | {p['version']} | {p.get('license', 'See notice')} |")
    npm.extend(["\n" + "=" * 72, f"{p['name']} {p['version']} ({p.get('license', 'See notice')})"])
    for file in texts:
        npm.extend([file.name, file.read_text().strip()])
(output / "NPM_NOTICES.txt").write_text("\n\n".join(npm) + "\n")
(output / "INVENTORY.md").write_text("\n".join(inventory) + "\n")
inputs = ("package-lock.json", "src-tauri/Cargo.lock", "about.toml")
(output / "lockfiles.json").write_text(json.dumps({name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in inputs}, indent=2) + "\n")
print(f"Generated {len(report['crates'])} Rust component entries and {len(paths) - 1} npm entries in docs/licenses")
