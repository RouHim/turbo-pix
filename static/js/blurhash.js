// BlurHash decoder for progressive image loading
// Based on the official blurhash algorithm
// https://github.com/woltapp/blurhash

const digitCharacters = [
  '0',
  '1',
  '2',
  '3',
  '4',
  '5',
  '6',
  '7',
  '8',
  '9',
  'A',
  'B',
  'C',
  'D',
  'E',
  'F',
  'G',
  'H',
  'I',
  'J',
  'K',
  'L',
  'M',
  'N',
  'O',
  'P',
  'Q',
  'R',
  'S',
  'T',
  'U',
  'V',
  'W',
  'X',
  'Y',
  'Z',
  'a',
  'b',
  'c',
  'd',
  'e',
  'f',
  'g',
  'h',
  'i',
  'j',
  'k',
  'l',
  'm',
  'n',
  'o',
  'p',
  'q',
  'r',
  's',
  't',
  'u',
  'v',
  'w',
  'x',
  'y',
  'z',
  '#',
  '$',
  '%',
  '*',
  '+',
  ',',
  '-',
  '.',
  ':',
  ';',
  '=',
  '?',
  '@',
  '[',
  ']',
  '^',
  '_',
  '{',
  '|',
  '}',
  '~',
];

const decode83 = (str) => {
  let value = 0;
  for (let i = 0; i < str.length; i++) {
    const c = str[i];
    const digit = digitCharacters.indexOf(c);
    if (digit === -1) throw new Error(`Invalid character in blurhash: ${c}`);
    value = value * 83 + digit;
  }
  return value;
};

const decodeDC = (value) => {
  const intR = value >> 16;
  const intG = (value >> 8) & 255;
  const intB = value & 255;
  return [sRGBToLinear(intR), sRGBToLinear(intG), sRGBToLinear(intB)];
};

const decodeAC = (value, maximumValue) => {
  const quantR = Math.floor(value / (19 * 19));
  const quantG = Math.floor(value / 19) % 19;
  const quantB = value % 19;

  return [
    signPow((quantR - 9) / 9, 2.0) * maximumValue,
    signPow((quantG - 9) / 9, 2.0) * maximumValue,
    signPow((quantB - 9) / 9, 2.0) * maximumValue,
  ];
};

const sRGBToLinear = (value) => {
  const v = value / 255;
  return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
};

const linearTosRGB = (value) => {
  const v = Math.max(0, Math.min(1, value));
  return v <= 0.0031308
    ? Math.round(v * 12.92 * 255 + 0.5)
    : Math.round((1.055 * Math.pow(v, 1 / 2.4) - 0.055) * 255 + 0.5);
};

const signPow = (value, exp) => {
  return Math.sign(value) * Math.pow(Math.abs(value), exp);
};

/**
 * Decodes a blurhash string to pixel data
 * @param {string} blurhash - The blurhash string
 * @param {number} width - Output width
 * @param {number} height - Output height
 * @param {number} punch - Contrast adjustment (default: 1)
 * @returns {Uint8ClampedArray} Pixel data in RGBA format
 */
const decode = (blurhash, width, height, punch = 1) => {
  if (!blurhash || blurhash.length < 6) {
    throw new Error('Invalid blurhash');
  }

  const sizeFlag = decode83(blurhash[0]);
  const numY = Math.floor(sizeFlag / 9) + 1;
  const numX = (sizeFlag % 9) + 1;

  const quantisedMaximumValue = decode83(blurhash[1]);
  const maximumValue = (quantisedMaximumValue + 1) / 166;

  const colors = new Array(numX * numY);

  for (let i = 0; i < colors.length; i++) {
    if (i === 0) {
      const value = decode83(blurhash.substring(2, 6));
      colors[i] = decodeDC(value);
    } else {
      const value = decode83(blurhash.substring(4 + i * 2, 6 + i * 2));
      colors[i] = decodeAC(value, maximumValue * punch);
    }
  }

  const pixels = new Uint8ClampedArray(width * height * 4);

  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      let r = 0;
      let g = 0;
      let b = 0;

      for (let j = 0; j < numY; j++) {
        for (let i = 0; i < numX; i++) {
          const basis = Math.cos((Math.PI * x * i) / width) * Math.cos((Math.PI * y * j) / height);
          const color = colors[i + j * numX];
          r += color[0] * basis;
          g += color[1] * basis;
          b += color[2] * basis;
        }
      }

      const idx = (y * width + x) * 4;
      pixels[idx] = linearTosRGB(r);
      pixels[idx + 1] = linearTosRGB(g);
      pixels[idx + 2] = linearTosRGB(b);
      pixels[idx + 3] = 255; // Alpha
    }
  }

  return pixels;
};

/**
 * Creates a canvas with the decoded blurhash
 * @param {string} blurhash - The blurhash string
 * @param {number} width - Canvas width
 * @param {number} height - Canvas height
 * @param {number} punch - Contrast adjustment (default: 1)
 * @returns {HTMLCanvasElement} Canvas element with the decoded image
 */
const createCanvas = (blurhash, width, height, punch = 1) => {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;

  const ctx = canvas.getContext('2d');
  const imageData = ctx.createImageData(width, height);
  const pixels = decode(blurhash, width, height, punch);

  imageData.data.set(pixels);
  ctx.putImageData(imageData, 0, 0);

  return canvas;
};

/**
 * Creates a data URL from a blurhash
 * @param {string} blurhash - The blurhash string
 * @param {number} width - Image width
 * @param {number} height - Image height
 * @param {number} punch - Contrast adjustment (default: 1)
 * @returns {string} Data URL
 */
const toDataURL = (blurhash, width, height, punch = 1) => {
  const canvas = createCanvas(blurhash, width, height, punch);
  return canvas.toDataURL();
};

// Export for use in other modules
window.blurhash = {
  decode,
  createCanvas,
  toDataURL,
};
