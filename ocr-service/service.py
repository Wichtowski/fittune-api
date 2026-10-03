import io
import json
import multiprocessing as mp
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MAX_BYTES = 4 * 1024 * 1024
TIMEOUT = 19


def recognize_worker(connection):
    import numpy as np
    import onnxruntime as ort
    ort.disable_telemetry_events()
    from PIL import Image
    from rapidocr import RapidOCR, OCRVersion, ModelType, LangRec

    engine = RapidOCR(params={
        "Det.limit_type": "max", "Det.limit_side_len": 960,
        "Rec.rec_batch_num": 1, "Cls.cls_batch_num": 1,
        "Det.model_path": "models/det.onnx", "Det.ocr_version": OCRVersion.PPOCRV5, "Det.model_type": ModelType.MOBILE,
        "Rec.model_path": "models/rec.onnx", "Rec.ocr_version": OCRVersion.PPOCRV5, "Rec.lang_type": LangRec.LATIN, "Rec.model_type": ModelType.MOBILE,
        "Cls.model_path": "models/cls.onnx",
        "EngineConfig.onnxruntime.intra_op_num_threads": 1,
        "EngineConfig.onnxruntime.inter_op_num_threads": 1,
        "Global.max_side_len": 960, "Global.log_level": "critical", "Global.return_word_box": True,
    })
    connection.send({"ready": True})
    while True:
        raw = connection.recv_bytes(MAX_BYTES)
        try:
            with Image.open(io.BytesIO(raw)) as photo:
                w, h = photo.size
                if max(w, h) > 2048 or w * h > 4_000_000 or photo.format not in ("PNG", "JPEG", "WEBP"):
                    raise ValueError("Invalid crop")
                rgb = np.array(photo.convert("RGB"))
            result = engine(rgb)
            observations = []
            # Word boxes keep both engines on the same parser contract
            for line in (result.word_results or []) if result.txts is not None else []:
                for text, score, box in line:
                    if box is None:
                        continue
                    x0, y0 = np.min(box, axis=0)
                    x1, y1 = np.max(box, axis=0)
                    bbox = [max(0, float(x0)), max(0, float(y0)), min(w, float(x1)), min(h, float(y1))]
                    if bbox[2] > bbox[0] and bbox[3] > bbox[1]:
                        observations.append({"text": text, "confidence": max(0, min(1, float(score))), "bbox": bbox})
            if len(observations) > 1000:
                raise ValueError("Too much text")
            payload = {"width": w, "height": h, "observations": observations}
            if len(json.dumps(payload).encode()) > 64 * 1024:
                raise ValueError("Too much text")
            connection.send(payload)
        except Exception:
            connection.send({"error": "Could not recognize crop"})


class Worker:
    def __init__(self, target=recognize_worker):
        self.target = target
        self.lock = threading.Lock()
        self.ready = False
        self.start()

    def start(self):
        self.ready = False
        self.connection, child = mp.get_context("spawn").Pipe()
        self.process = mp.get_context("spawn").Process(target=self.target, args=(child,), daemon=True)
        self.process.start()
        child.close()

    def stop(self):
        self.process.kill()
        self.process.join(timeout=2)
        self.connection.close()

    def health(self):
        if not self.process.is_alive():
            self.stop()
            self.start()
            return False
        if not self.ready and self.connection.poll():
            self.ready = self.connection.recv().get("ready", False)
        return self.ready

    def run(self, data, timeout=TIMEOUT):
        if not self.lock.acquire(blocking=False):
            raise RuntimeError("OCR is busy")
        try:
            started = time.monotonic()
            if not self.health():
                if not self.connection.poll(min(3, timeout)) or not self.health():
                    raise RuntimeError("OCR is warming up")
            self.connection.send_bytes(data)
            if not self.connection.poll(max(0, timeout - (time.monotonic() - started))):
                self.stop()
                self.start()
                raise RuntimeError("OCR timed out")
            result = self.connection.recv()
            if "error" in result:
                raise RuntimeError(result["error"])
            return result
        except (EOFError, BrokenPipeError, OSError) as failure:
            self.process.join(timeout=0.5)
            print(json.dumps({"event":"ocr_worker_failed","error_type":type(failure).__name__,"exit_code":self.process.exitcode}), flush=True)
            self.stop()
            self.start()
            raise RuntimeError("OCR worker failed") from None
        finally:
            self.lock.release()


class Handler(BaseHTTPRequestHandler):
    worker: Worker

    def log_message(self, *args):
        pass

    def reply(self, status, body):
        raw = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        try:
            self.wfile.write(raw)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        if self.path != "/health":
            return self.reply(404, {"error": "Not found"})
        if not self.worker.lock.acquire(blocking=False):
            return self.reply(200, {"status": "busy"})
        try:
            ready = self.worker.health()
        finally:
            self.worker.lock.release()
        self.reply(200 if ready else 503, {"ready": ready})

    def do_POST(self):
        if self.path != "/recognize":
            return self.reply(404, {"error": "Not found"})
        self.connection.settimeout(5)
        try:
            size = int(self.headers.get("Content-Length", "0"))
            if not 0 < size <= MAX_BYTES or self.headers.get("Content-Type") not in ("image/png", "image/jpeg"):
                return self.reply(413, {"error": "Invalid crop upload"})
            raw = self.rfile.read(size)
            if len(raw) != size:
                return self.reply(400, {"error": "Incomplete crop"})
            self.reply(200, self.worker.run(raw))
        except (ValueError, TimeoutError):
            self.reply(400, {"error": "Invalid crop upload"})
        except RuntimeError as error:
            self.reply(503, {"error": str(error)})


if __name__ == "__main__":
    Handler.worker = Worker()
    ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
