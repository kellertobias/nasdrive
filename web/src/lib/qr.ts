type QrVersion = 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9;

interface QrSpec {
  version: QrVersion;
  size: number;
  dataCodewords: number;
  eccCodewords: number;
  blocks: number;
}

const specs: QrSpec[] = [
  { version: 1, size: 21, dataCodewords: 19, eccCodewords: 7, blocks: 1 },
  { version: 2, size: 25, dataCodewords: 34, eccCodewords: 10, blocks: 1 },
  { version: 3, size: 29, dataCodewords: 55, eccCodewords: 15, blocks: 1 },
  { version: 4, size: 33, dataCodewords: 80, eccCodewords: 20, blocks: 1 },
  { version: 5, size: 37, dataCodewords: 108, eccCodewords: 26, blocks: 1 },
  { version: 6, size: 41, dataCodewords: 136, eccCodewords: 18, blocks: 2 },
  { version: 7, size: 45, dataCodewords: 156, eccCodewords: 20, blocks: 2 },
  { version: 8, size: 49, dataCodewords: 194, eccCodewords: 24, blocks: 2 },
  { version: 9, size: 53, dataCodewords: 232, eccCodewords: 30, blocks: 2 },
];

export function qrSvgDataUrl(text: string, scale = 4, quiet = 4): string {
  const modules = createQrModules(text);
  const size = modules.length + quiet * 2;
  const rects: string[] = [];
  for (let y = 0; y < modules.length; y += 1) {
    for (let x = 0; x < modules.length; x += 1) {
      if (modules[y][x]) {
        rects.push(
          `<rect x="${x + quiet}" y="${y + quiet}" width="1" height="1"/>`,
        );
      }
    }
  }
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${size * scale}" height="${size * scale}" viewBox="0 0 ${size} ${size}" shape-rendering="crispEdges"><rect width="100%" height="100%" fill="#fff"/><g fill="#000">${rects.join("")}</g></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function createQrModules(text: string): boolean[][] {
  const data = new TextEncoder().encode(text);
  const spec = specs.find(
    (candidate) => data.length + 2 <= candidate.dataCodewords,
  );
  if (!spec) throw new Error("QR payload is too long");

  const codewords = encodeData(data, spec);
  const allCodewords = addErrorCorrection(codewords, spec);
  const matrix = Array.from({ length: spec.size }, () =>
    Array<boolean | null>(spec.size).fill(null),
  );
  const reserved = Array.from({ length: spec.size }, () =>
    Array(spec.size).fill(false),
  );

  drawFunctionPatterns(matrix, reserved, spec.version);
  drawCodewords(matrix, reserved, allCodewords);
  applyMask(matrix, reserved);
  drawFormatBits(matrix);

  return matrix.map((row) => row.map(Boolean));
}

function encodeData(data: Uint8Array, spec: QrSpec): number[] {
  const bits: number[] = [];
  appendBits(bits, 0b0100, 4);
  appendBits(bits, data.length, spec.version <= 9 ? 8 : 16);
  for (const byte of data) appendBits(bits, byte, 8);
  const capacityBits = spec.dataCodewords * 8;
  appendBits(bits, 0, Math.min(4, capacityBits - bits.length));
  while (bits.length % 8 !== 0) bits.push(0);

  const codewords: number[] = [];
  for (let i = 0; i < bits.length; i += 8) {
    codewords.push(
      bits.slice(i, i + 8).reduce((value, bit) => (value << 1) | bit, 0),
    );
  }
  for (
    let pad = 0xec;
    codewords.length < spec.dataCodewords;
    pad = pad === 0xec ? 0x11 : 0xec
  ) {
    codewords.push(pad);
  }
  return codewords;
}

function addErrorCorrection(data: number[], spec: QrSpec): number[] {
  const blockLength = spec.dataCodewords / spec.blocks;
  if (!Number.isInteger(blockLength))
    throw new Error("Unsupported QR block layout");

  const blocks: number[][] = [];
  const eccBlocks: number[][] = [];
  for (let i = 0; i < spec.blocks; i += 1) {
    const block = data.slice(i * blockLength, (i + 1) * blockLength);
    blocks.push(block);
    eccBlocks.push(reedSolomonRemainder(block, spec.eccCodewords));
  }

  const result: number[] = [];
  for (let i = 0; i < blockLength; i += 1) {
    for (const block of blocks) result.push(block[i]);
  }
  for (let i = 0; i < spec.eccCodewords; i += 1) {
    for (const block of eccBlocks) result.push(block[i]);
  }
  return result;
}

function drawFunctionPatterns(
  matrix: (boolean | null)[][],
  reserved: boolean[][],
  version: QrVersion,
) {
  const size = matrix.length;
  drawFinder(matrix, reserved, 0, 0);
  drawFinder(matrix, reserved, size - 7, 0);
  drawFinder(matrix, reserved, 0, size - 7);

  for (let i = 0; i < size; i += 1) {
    setReserved(matrix, reserved, 6, i, i % 2 === 0);
    setReserved(matrix, reserved, i, 6, i % 2 === 0);
  }

  const alignments = alignmentPositions(version);
  for (const y of alignments) {
    for (const x of alignments) {
      if (reserved[y][x]) continue;
      drawAlignment(matrix, reserved, x, y);
    }
  }

  setReserved(matrix, reserved, 8, size - 8, true);
  for (let i = 0; i < 9; i += 1) {
    setReserved(matrix, reserved, 8, i, false);
    setReserved(matrix, reserved, i, 8, false);
    setReserved(matrix, reserved, size - 1 - i, 8, false);
    setReserved(matrix, reserved, 8, size - 1 - i, false);
  }
}

function drawFinder(
  matrix: (boolean | null)[][],
  reserved: boolean[][],
  left: number,
  top: number,
) {
  for (let y = -1; y <= 7; y += 1) {
    for (let x = -1; x <= 7; x += 1) {
      const xx = left + x;
      const yy = top + y;
      if (yy < 0 || yy >= matrix.length || xx < 0 || xx >= matrix.length)
        continue;
      const dark =
        x >= 0 &&
        x <= 6 &&
        y >= 0 &&
        y <= 6 &&
        (x === 0 ||
          x === 6 ||
          y === 0 ||
          y === 6 ||
          (x >= 2 && x <= 4 && y >= 2 && y <= 4));
      setReserved(matrix, reserved, xx, yy, dark);
    }
  }
}

function drawAlignment(
  matrix: (boolean | null)[][],
  reserved: boolean[][],
  cx: number,
  cy: number,
) {
  for (let y = -2; y <= 2; y += 1) {
    for (let x = -2; x <= 2; x += 1) {
      setReserved(
        matrix,
        reserved,
        cx + x,
        cy + y,
        Math.max(Math.abs(x), Math.abs(y)) !== 1,
      );
    }
  }
}

function drawCodewords(
  matrix: (boolean | null)[][],
  reserved: boolean[][],
  codewords: number[],
) {
  const bits = codewords.flatMap((byte) =>
    Array.from({ length: 8 }, (_, i) => ((byte >> (7 - i)) & 1) === 1),
  );
  let bitIndex = 0;
  let upward = true;
  for (let right = matrix.length - 1; right > 0; right -= 2) {
    if (right === 6) right -= 1;
    for (let vert = 0; vert < matrix.length; vert += 1) {
      const y = upward ? matrix.length - 1 - vert : vert;
      for (let dx = 0; dx < 2; dx += 1) {
        const x = right - dx;
        if (reserved[y][x]) continue;
        matrix[y][x] = bits[bitIndex] ?? false;
        bitIndex += 1;
      }
    }
    upward = !upward;
  }
}

function applyMask(matrix: (boolean | null)[][], reserved: boolean[][]) {
  for (let y = 0; y < matrix.length; y += 1) {
    for (let x = 0; x < matrix.length; x += 1) {
      if (!reserved[y][x] && (x + y) % 2 === 0) matrix[y][x] = !matrix[y][x];
    }
  }
}

function drawFormatBits(matrix: (boolean | null)[][]) {
  const bits = 0b111011111000100; // ECC level L, mask 0.
  const size = matrix.length;
  for (let i = 0; i < 15; i += 1) {
    const bit = ((bits >> i) & 1) === 1;
    const a = formatCoordinateA(i);
    const b = formatCoordinateB(i, size);
    matrix[a.y][a.x] = bit;
    matrix[b.y][b.x] = bit;
  }
}

function formatCoordinateA(i: number) {
  if (i < 6) return { x: 8, y: i };
  if (i < 8) return { x: 8, y: i + 1 };
  if (i === 8) return { x: 7, y: 8 };
  return { x: 14 - i, y: 8 };
}

function formatCoordinateB(i: number, size: number) {
  if (i < 8) return { x: size - 1 - i, y: 8 };
  return { x: 8, y: size - 15 + i };
}

function setReserved(
  matrix: (boolean | null)[][],
  reserved: boolean[][],
  x: number,
  y: number,
  value: boolean,
) {
  if (y < 0 || y >= matrix.length || x < 0 || x >= matrix.length) return;
  matrix[y][x] = value;
  reserved[y][x] = true;
}

function appendBits(bits: number[], value: number, length: number) {
  for (let i = length - 1; i >= 0; i -= 1) bits.push((value >> i) & 1);
}

function alignmentPositions(version: QrVersion): number[] {
  const positions: Record<QrVersion, number[]> = {
    1: [],
    2: [6, 18],
    3: [6, 22],
    4: [6, 26],
    5: [6, 30],
    6: [6, 34],
    7: [6, 22, 38],
    8: [6, 24, 42],
    9: [6, 26, 46],
  };
  return positions[version];
}

function reedSolomonRemainder(data: number[], degree: number): number[] {
  const generator = reedSolomonGenerator(degree);
  const result = Array(degree).fill(0);
  for (const byte of data) {
    const factor = byte ^ (result.shift() ?? 0);
    result.push(0);
    for (let i = 0; i < degree; i += 1) {
      result[i] ^= gfMultiply(generator[i], factor);
    }
  }
  return result;
}

function reedSolomonGenerator(degree: number): number[] {
  const result = [1];
  for (let i = 0; i < degree; i += 1) {
    result.push(0);
    for (let j = result.length - 1; j > 0; j -= 1) {
      result[j] = result[j - 1] ^ gfMultiply(result[j], gfPow(2, i));
    }
    result[0] = gfMultiply(result[0], gfPow(2, i));
  }
  return result.slice(1);
}

function gfMultiply(x: number, y: number): number {
  let result = 0;
  for (; y > 0; y >>= 1) {
    if (y & 1) result ^= x;
    x <<= 1;
    if (x & 0x100) x ^= 0x11d;
  }
  return result;
}

function gfPow(x: number, power: number): number {
  let result = 1;
  for (let i = 0; i < power; i += 1) result = gfMultiply(result, x);
  return result;
}
