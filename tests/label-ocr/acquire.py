"""Acquire public, attributed fixture candidates without uploading user photos"""
import argparse
import concurrent.futures
import hashlib
import json
import time
from pathlib import Path
from urllib.request import Request, urlopen


def fetch(url):
    for attempt in range(4):
        try:
            with urlopen(Request(url, headers={"User-Agent": "FitTune-label-fixtures/1.0 (public benchmark)"}), timeout=60) as response:
                return response.read(16 * 1024 * 1024)
        except Exception:
            if attempt == 3:
                raise
            time.sleep(2 ** attempt)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    committed = Path(__file__).with_name("manifest.json")
    manifest = json.loads(committed.read_text())
    def restore(entry):
        destination = args.output / (entry["id"] + ".jpg")
        data = destination.read_bytes() if destination.exists() else fetch(entry["image_url"])
        if hashlib.sha256(data).hexdigest() != entry["sha256"]:
            raise RuntimeError("Fixture checksum changed: " + entry["id"])
        destination.write_bytes(data)
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
        list(executor.map(restore, manifest["entries"]))
    (args.output / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print("Restored 60 checksum-verified fixtures")


if __name__ == "__main__":
    main()
