// Decodes an 8-bit RGBA, non-interlaced PNG (the combat-effects atlases, asset-tooling
// `docs/combat-effects.md`) to top-row-first straight-alpha RGBA bytes. Decoding the file
// itself keeps every texel exact: a canvas round trip premultiplies alpha and would drop
// the colour the atlases keep in fully transparent texels. Any other PNG kind is rejected.

const SIGNATURE = [137, 80, 78, 71, 13, 10, 26, 10];

async function inflate(bytes) {
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("deflate"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

const paeth = (a, b, c) => {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
};

export async function decodePngRgba(input) {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  if (!SIGNATURE.every((value, index) => bytes[index] === value)) throw new Error("Not a PNG");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let width = 0;
  let height = 0;
  const data = [];
  for (let offset = 8; offset + 8 <= bytes.length;) {
    const length = view.getUint32(offset);
    const type = String.fromCharCode(...bytes.subarray(offset + 4, offset + 8));
    const body = bytes.subarray(offset + 8, offset + 8 + length);
    if (type === "IHDR") {
      width = view.getUint32(offset + 8);
      height = view.getUint32(offset + 12);
      const [depth, colour, , , interlace] = body.subarray(8, 13);
      if (depth !== 8 || colour !== 6 || interlace !== 0)
        throw new Error("Only 8-bit RGBA non-interlaced PNGs are supported");
    } else if (type === "IDAT") data.push(body);
    else if (type === "IEND") break;
    offset += 12 + length;
  }
  if (!width || !height || !data.length) throw new Error("Incomplete PNG");
  const raw = await inflate(new Uint8Array(await new Blob(data).arrayBuffer()));
  const stride = width * 4;
  if (raw.length < (stride + 1) * height) throw new Error("Truncated PNG image data");
  const pixels = new Uint8Array(stride * height);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const source = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const row = y * stride;
    for (let x = 0; x < stride; x += 1) {
      const left = x >= 4 ? pixels[row + x - 4] : 0;
      const up = y > 0 ? pixels[row - stride + x] : 0;
      const corner = x >= 4 && y > 0 ? pixels[row - stride + x - 4] : 0;
      const predictor =
        filter === 0
          ? 0
          : filter === 1
            ? left
            : filter === 2
              ? up
              : filter === 3
                ? (left + up) >> 1
                : filter === 4
                  ? paeth(left, up, corner)
                  : -1;
      if (predictor < 0) throw new Error(`Unknown PNG filter ${filter}`);
      pixels[row + x] = (source[x] + predictor) & 0xff;
    }
  }
  return { width, height, pixels };
}
