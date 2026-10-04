"""Run OCR fixtures and replay normalized observations through the Rust parser"""
import argparse
import io
import json
import math
import time
from decimal import Decimal, ROUND_CEILING
from urllib.error import HTTPError
from pathlib import Path
from urllib.request import Request, urlopen

from PIL import Image, ImageOps
import numpy as np


def crop(entry, root):
    photo = ImageOps.exif_transpose(Image.open(root / (entry["id"] + ".jpg")))
    w, h = photo.size
    box = entry["crop"]
    photo = photo.crop(tuple(int(v * size) for v, size in zip(box, [w, h, w, h])))
    photo = photo.rotate(entry["rotation"], expand=True)
    w, h = photo.size
    scale = min(1, 2048 / max(w, h), math.sqrt(4_000_000 / (w * h)))
    return photo.resize((max(1, int(w * scale)), max(1, int(h * scale))), Image.Resampling.LANCZOS).convert("RGB")


def preprocess(photo, variant):
    if variant == "original": return photo
    gray = photo.convert("L")
    if variant == "grayscale": return gray.convert("RGB")
    values = np.array(gray, dtype=np.float64)
    if variant == "otsu":
        histogram = np.bincount(values.astype(np.uint8).ravel(), minlength=256).astype(np.float64)
        weight = np.cumsum(histogram)
        mean = np.cumsum(histogram * np.arange(256))
        denominator = weight * (weight[-1] - weight)
        score = np.divide((mean[-1] * weight - mean * weight[-1]) ** 2, denominator, out=np.zeros(256), where=denominator > 0)
        threshold = np.argmax(score)
    else:
        def local_mean(data):
            padded = np.pad(data, 12, mode="reflect")
            integral = np.pad(padded.cumsum(0).cumsum(1), ((1,0),(1,0)))
            return (integral[25:,25:] - integral[:-25,25:] - integral[25:,:-25] + integral[:-25,:-25]) / 625
        mean = local_mean(values)
        deviation = np.sqrt(np.maximum(0, local_mean(values * values) - mean * mean))
        threshold = mean * (1 + .2 * (deviation / 128 - 1))
    return Image.fromarray(np.where(values > threshold, 255, 0).astype(np.uint8)).convert("RGB")

def request(url, body, token=None, content_type="application/json"):
    headers = {"Content-Type": content_type}
    if token:
        headers["Authorization"] = "Bearer " + token
    with urlopen(Request(url, data=body, headers=headers), timeout=30) as response:
        return json.load(response)


SCORING = {"rounding": "ceiling", "decimal_places": 1, "absolute_tolerance": 0.6, "require_same_basis": True}


def rounded(value):
    return Decimal(str(round(value, 12))).quantize(Decimal("0.1"), rounding=ROUND_CEILING)


def matches(expected, actual):
    if expected is None or actual is None:
        return expected is None and actual is None
    if not math.isfinite(actual):
        return False
    return abs(rounded(expected) - rounded(actual)) <= Decimal("0.6")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--observations", type=Path, required=True)
    parser.add_argument("--api")
    parser.add_argument("--token-file", type=Path)
    parser.add_argument("--rapid-url")
    parser.add_argument("--ids", help="Comma-separated fixture IDs for diagnosis")
    parser.add_argument("--split", choices=["development", "heldout", "all"], default="development")
    parser.add_argument("--report", type=Path)
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--variant", choices=["original", "grayscale", "otsu", "sauvola"], default="original")
    args = parser.parse_args()
    token = args.token_file.read_text().strip() if args.token_file else None
    if not args.prepare_only and not (args.api and token and args.report): parser.error("--api, --token-file and --report are required for evaluation")
    entries = json.loads(Path(__file__).with_name("manifest.json").read_text())["entries"]
    args.observations.mkdir(parents=True, exist_ok=True)
    results = []
    for entry in entries:
        if args.ids and entry["id"] not in args.ids.split(","): continue
        if args.split != "all" and entry["split"] != args.split:
            continue
        photo = preprocess(crop(entry, args.fixtures), args.variant)
        photo.save(args.observations / (entry["id"] + ".jpg"), quality=92)
        cache = args.observations / (entry["id"] + ".json")
        if args.prepare_only:
            cache.write_text(json.dumps({"width":photo.width,"height":photo.height}))
            continue
        elapsed = None
        failure = None
        if args.rapid_url:
            started = time.monotonic()
            for _ in range(10):
                try:
                    with urlopen(args.rapid_url.rstrip("/") + "/health", timeout=3):
                        break
                except Exception:
                    time.sleep(1)
            raw = io.BytesIO()
            photo.save(raw, format="JPEG", quality=92)
            try:
                observations = request(args.rapid_url.rstrip("/") + "/recognize", raw.getvalue(), content_type="image/jpeg")
            except HTTPError as error:
                elapsed = time.monotonic() - started
                failure = error.code
                observations = {"width": photo.width, "height": photo.height, "observations": []}
                print(entry["id"], "server failure", error.code, error.read().decode(), flush=True)
            elapsed = time.monotonic() - started
            cache.write_text(json.dumps(observations, ensure_ascii=False) + "\n")
            cache.with_suffix(".timing.json").write_text(json.dumps({"seconds":elapsed,"failure":failure}))
        else:
            observations = json.loads(cache.read_text())
            timing = cache.with_suffix(".timing.json")
            if timing.exists():
                metadata = json.loads(timing.read_text()); elapsed = metadata["seconds"]; failure = metadata.get("failure")
        result = request(args.api.rstrip("/") + "/api/v1/health/ocr/parse", json.dumps(observations).encode(), token)
        if entry.get("column_unit") and result["selected_column"] is None:
            columns = [i for i, c in enumerate(result["columns"]) if c["unit"] == entry["column_unit"]]
            if len(columns) == 1:
                observations["column"] = columns[0]
                result = request(args.api.rstrip("/") + "/api/v1/health/ocr/parse", json.dumps(observations).encode(), token)
        differences = {}
        correct = total = proposed = wrong = 0
        for field, truth in entry["expected"]["values"].items():
            actual = result["values"][field]
            equal = matches(truth, actual)
            if truth is not None:
                total += 1
                correct += int(equal and result["unit"] == entry["expected"]["unit"])
            if actual is not None:
                proposed += 1
                wrong += int(not equal or result["unit"] != entry["expected"]["unit"])
            if not equal:
                differences[field] = {"expected": truth, "actual": actual, "rounded_expected": float(rounded(truth)) if truth is not None else None, "rounded_actual": float(rounded(actual)) if actual is not None else None}
        if result["unit"] != entry["expected"]["unit"]:
            differences["unit"] = {"expected": entry["expected"]["unit"], "actual": result["unit"]}
        results.append({"id": entry["id"], "language": entry["language"], "split": entry["split"], "difficulty": entry["difficulty"], "seconds": elapsed, "failure":failure, "correct": correct, "total": total, "proposed": proposed, "wrong": wrong, "differences": differences, "result": result})
        print(entry["id"], f"{correct}/{total}, {wrong} incorrect suggestions", flush=True)
    if args.prepare_only: return
    summary = {}
    for lang in ["pl", "en"]:
        rows = [r for r in results if r["language"] == lang]
        timings = sorted(r["seconds"] for r in rows if r["seconds"] is not None)
        summary[lang] = {"failed":sum(r["failure"] is not None for r in rows), "correct": sum(r["correct"] for r in rows), "total": sum(r["total"] for r in rows), "proposed": sum(r["proposed"] for r in rows), "wrong": sum(r["wrong"] for r in rows), "p95_seconds": timings[max(0, math.ceil(len(timings) * .95) - 1)] if timings else None}
    args.report.write_text(json.dumps({"variant":args.variant,"split":args.split,"scoring":SCORING,"summary": summary, "results": results}, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
