import "@testing-library/jest-dom/vitest";

// jsdom lacks several browser APIs that Radix UI primitives (Slider, Select,
// Tooltip) touch on mount. Polyfill the minimum so component tests can render.
if (typeof globalThis.ResizeObserver === "undefined") {
  class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver = ResizeObserver as unknown as typeof globalThis.ResizeObserver;
}

if (!("PointerEvent" in globalThis)) {
  // Minimal stub; Radix only reads a few properties.
  globalThis.PointerEvent = class PointerEvent extends Event {} as unknown as typeof globalThis.PointerEvent;
}

for (const fn of ["scrollIntoView", "hasPointerCapture", "releasePointerCapture", "setPointerCapture"] as const) {
  if (typeof Element !== "undefined" && !(fn in Element.prototype)) {
    // @ts-expect-error -- assigning test-only no-op stubs
    Element.prototype[fn] = () => {};
  }
}
