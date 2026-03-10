import "@testing-library/jest-dom/vitest";

if (!("createObjectURL" in URL)) {
  Object.defineProperty(URL, "createObjectURL", {
    value: () => "blob:mock",
    writable: true,
  });
}

if (!("revokeObjectURL" in URL)) {
  Object.defineProperty(URL, "revokeObjectURL", {
    value: () => {},
    writable: true,
  });
}

Object.defineProperty(HTMLMediaElement.prototype, "play", {
  configurable: true,
  value: async () => {},
  writable: true,
});

Object.defineProperty(HTMLMediaElement.prototype, "pause", {
  configurable: true,
  value: () => {},
  writable: true,
});

Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
  configurable: true,
  value: () => ({
    clearRect: () => {},
    fillRect: () => {},
    setTransform: () => {},
    fillStyle: "",
  }),
  writable: true,
});
