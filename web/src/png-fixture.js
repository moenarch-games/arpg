// Test fixture: encodes RGBA rows as a PNG with the given per-row filter types (chunk CRCs
// are left zero; the decoder does not check them).
import { deflateSync } from "node:zlib";

const paeth = (a, b, c) => {
  const p = a + b - c;
  const [pa, pb, pc] = [Math.abs(p - a), Math.abs(p - b), Math.abs(p - c)];
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
};

export function encodePng(width, height, pixels, { filters, colour = 6 } = {}) {
  const stride = width * 4;
  const raw = [];
  for (let y = 0; y < height; y += 1) {
    const filter = filters?.[y] ?? 0;
    raw.push(filter);
    for (let x = 0; x < stride; x += 1) {
      const at = (dx, dy) => (x + dx >= 0 && y + dy >= 0 ? pixels[(y + dy) * stride + x + dx] : 0);
      const [left, up, corner] = [at(-4, 0), at(0, -1), at(-4, -1)];
      const predictor = [0, left, up, (left + up) >> 1, paeth(left, up, corner)][filter];
      raw.push((pixels[y * stride + x] - predictor) & 0xff);
    }
  }
  const chunk = (type, body) => {
    const out = new Uint8Array(12 + body.length);
    new DataView(out.buffer).setUint32(0, body.length);
    out.set(new TextEncoder().encode(type), 4);
    out.set(body, 8);
    return out;
  };
  const header = new Uint8Array(13);
  new DataView(header.buffer).setUint32(0, width);
  new DataView(header.buffer).setUint32(4, height);
  header.set([8, colour, 0, 0, 0], 8);
  return new Uint8Array(
    Buffer.concat([
      new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
      chunk("IHDR", header),
      chunk("IDAT", deflateSync(new Uint8Array(raw))),
      chunk("IEND", new Uint8Array()),
    ]),
  );
}
