import hashlib
from pathlib import Path
from urllib.request import urlopen

MODELS = {
    "det.onnx": ("PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx", "4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae"),
    "rec.onnx": ("PP-OCRv5/rec/latin_PP-OCRv5_rec_mobile.onnx", "b20bd37c168a570f583afbc8cd7925603890efbcdc000a59e22c269d160b5f5a"),
    "cls.onnx": ("PP-OCRv4/cls/ch_ppocr_mobile_v2.0_cls_mobile.onnx", "e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c"),
}

if __name__ == "__main__":
    Path("models").mkdir(exist_ok=True)
    for name, (path, sha) in MODELS.items():
        url = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.2/onnx/" + path
        with urlopen(url, timeout=120) as response:
            data = response.read(64 * 1024 * 1024)
        if hashlib.sha256(data).hexdigest() != sha:
            raise RuntimeError("Model checksum mismatch: " + name)
        Path("models", name).write_bytes(data)
