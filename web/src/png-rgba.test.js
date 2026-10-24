import { expect, test } from "bun:test";
import { encodePng } from "./png-fixture.js";
import { decodePngRgba } from "./png-rgba.js";

test("decodes every row filter to exact straight-alpha RGBA, keeping transparent colour", async () => {
  const width = 3;
  const height = 5;
  const pixels = Uint8Array.from({ length: width * height * 4 }, (_, i) => (i * 37 + 11) % 256);
  pixels[3] = 0; // A fully transparent texel keeps its RGB.
  const decoded = await decodePngRgba(
    encodePng(width, height, pixels, { filters: [0, 1, 2, 3, 4] }),
  );
  expect(decoded.width).toBe(width);
  expect(decoded.height).toBe(height);
  expect([...decoded.pixels]).toEqual([...pixels]);
});

test("rejects non-RGBA PNGs and non-PNG bytes", async () => {
  await expect(decodePngRgba(encodePng(1, 1, new Uint8Array(4), { colour: 2 }))).rejects.toThrow(
    "8-bit RGBA",
  );
  await expect(decodePngRgba(new Uint8Array(16))).rejects.toThrow("Not a PNG");
});
