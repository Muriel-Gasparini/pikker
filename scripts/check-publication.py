"""Validate publication inputs. Development tool; not part of the app runtime."""
import argparse
import re
import subprocess
import tomllib
from pathlib import Path
from urllib.parse import unquote, urlsplit


def check(root, tag=None):
    package = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]
    if package.get("license") != "MIT" or package.get("repository") != "https://github.com/Muriel-Gasparini/pikker":
        raise ValueError("missing or inconsistent publication metadata")
    if tag is not None:
        number = r"(?:0|[1-9][0-9]*)"
        identifier = r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
        version = rf"{number}\.{number}\.{number}(?:-{identifier}(?:\.{identifier})*)?"
        if not re.fullmatch("v" + version, tag) or tag != "v" + package["version"]:
            raise ValueError("tag must be a valid version matching Cargo.toml")
    for name in ["LICENSE", "THIRD_PARTY_NOTICES.txt"]:
        if len((root / name).read_text(encoding="utf-8")) < 100:
            raise ValueError("empty distribution notice: " + name)
    tracked = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z"], text=True).split("\0")
    forbidden = {"profile.json", "settings.json", "meta.json", "lockfile", ".env"}
    for name in filter(None, tracked):
        path = Path(name)
        if path.name in forbidden or path.name.startswith(".env.") or name.startswith(("docs/superpowers/", "spikes/", "dist/", "target/", ".superpowers/")):
            raise ValueError("private/internal file tracked: " + name)
        if path.suffix != ".md":
            continue
        document = root / path
        text = document.read_text(encoding="utf-8")
        if re.search(r"/home/[^\s]+|/tmp/pikker[^\s]*", text):
            raise ValueError("personal/internal path in " + name)
        for target in re.findall(r"!?\[[^\]]*\]\(([^)]+)\)", text):
            target = target.strip().strip("<>")
            parsed = urlsplit(target)
            if parsed.scheme or parsed.netloc or not parsed.path:
                continue
            linked = (document.parent / unquote(parsed.path)).resolve()
            if not linked.is_relative_to(root.resolve()) or not linked.exists():
                raise ValueError("broken local link in " + name + ": " + target)
    return package["version"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--tag")
    args = parser.parse_args()
    try:
        print("Publication inputs valid:", check(args.root, args.tag))
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, str(error) + "\n")
