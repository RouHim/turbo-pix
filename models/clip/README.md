# CLIP Models Directory

This directory contains the CLIP models for semantic search.

## Download Models

Run the download script to fetch the models:

```bash
./scripts/download_clip_models.sh
```

## Model Information

- **Name:** nllb-clip-base-siglip__v1
- **Source:** [HuggingFace - immich-app](https://huggingface.co/immich-app/nllb-clip-base-siglip__v1)
- **Type:** Multilingual CLIP (100+ languages including German, English)
- **Size:** ~600MB total
  - `visual.onnx` - Visual encoder (~300MB)
  - `textual.onnx` - Text encoder (~300MB)
  - `tokenizer.json` - Tokenizer configuration

## Usage

After downloading, the models will be loaded automatically when CLIP search is enabled in the configuration.

## Note

These model files are not included in the repository due to their size. You must download them separately using the script provided.
