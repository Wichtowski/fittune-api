FROM python:3.12-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends libgl1 libglib2.0-0 && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir rapidocr==3.9.2 onnxruntime==1.30.0
COPY download_models.py .
RUN python download_models.py
COPY service.py .
RUN useradd --uid 10001 --no-create-home ocr
USER 10001:10001
ENV PYTHONDONTWRITEBYTECODE=1 OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1
EXPOSE 8080
CMD ["python", "service.py"]
