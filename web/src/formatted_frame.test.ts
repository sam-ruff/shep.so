import { it, expect } from "vitest";
import { readCanvas } from "./formatted_frame";

it("accepts only a lowercase hex canvas with a light or dark scheme", () => {
  expect(readCanvas({ background: "#ffffff", scheme: "light" })).toEqual({
    background: "#ffffff",
    scheme: "light",
  });
  expect(readCanvas({ background: "#18181b", scheme: "dark" })).toEqual({
    background: "#18181b",
    scheme: "dark",
  });
  for (const value of [
    {},
    { background: "white", scheme: "light" },
    { background: "#FFFFFF", scheme: "light" },
    { background: "#fff", scheme: "light" },
    { background: "url(https://images.example.test/x)", scheme: "dark" },
    { background: "#ffffff", scheme: "sepia" },
    { background: 16777215, scheme: "light" },
  ])
    expect(readCanvas(value)).toBeUndefined();
});
