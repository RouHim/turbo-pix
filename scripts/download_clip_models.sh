#!/bin/bash
set -e

echo "Downloading CLIP models for TurboPix..."
echo "Model: nllb-clip-base-siglip__v1 (multilingual)"
echo ""

# Create models directory if it doesn't exist
mkdir -p models/clip
cd models/clip

# Check if huggingface-cli is available
if ! command -v huggingface-cli &> /dev/null; then
    echo "huggingface-cli not found. Installing..."
    pip install --upgrade huggingface_hub
fi

# Download the model files
echo "Downloading visual encoder..."
huggingface-cli download immich-app/nllb-clip-base-siglip__v1 visual.onnx --local-dir .

echo "Downloading textual encoder..."
huggingface-cli download immich-app/nllb-clip-base-siglip__v1 textual.onnx --local-dir .

echo "Downloading tokenizer configuration..."
huggingface-cli download immich-app/nllb-clip-base-siglip__v1 tokenizer.json --local-dir .

echo ""
echo "✅ CLIP models downloaded successfully!"
echo "Location: $(pwd)"
echo ""
echo "Files:"
ls -lh *.onnx tokenizer.json 2>/dev/null || echo "Model files downloaded"

echo ""
echo "You can now enable CLIP search in your configuration:"
echo "  CLIP_ENABLE=true"
echo "  CLIP_MODEL_PATH=./models/clip"
