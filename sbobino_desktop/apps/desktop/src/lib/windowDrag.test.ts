import { describe, expect, it } from "vitest";
import { shouldStartWindowDrag } from "./windowDrag";

describe("window drag helpers", () => {
  it("allows dragging from plain decorative surfaces", () => {
    const surface = document.createElement("div");
    expect(shouldStartWindowDrag(surface)).toBe(true);
  });

  it("blocks dragging from interactive descendants", () => {
    const wrapper = document.createElement("div");
    const button = document.createElement("button");
    const icon = document.createElement("span");
    button.appendChild(icon);
    wrapper.appendChild(button);

    expect(shouldStartWindowDrag(icon)).toBe(false);
  });

  it("blocks dragging from explicit no-drag regions", () => {
    const region = document.createElement("div");
    region.setAttribute("data-tauri-drag-region", "false");

    expect(shouldStartWindowDrag(region)).toBe(false);
  });

  it("requires an explicit drag area when requested", () => {
    const outside = document.createElement("div");
    expect(shouldStartWindowDrag(outside, { requireExplicitArea: true })).toBe(false);

    const region = document.createElement("div");
    region.setAttribute("data-tauri-drag-region", "");
    expect(shouldStartWindowDrag(region, { requireExplicitArea: true })).toBe(true);
  });

  it("still blocks interactive controls inside an explicit drag area", () => {
    const region = document.createElement("div");
    region.setAttribute("data-tauri-drag-region", "");
    const button = document.createElement("button");
    region.appendChild(button);

    expect(shouldStartWindowDrag(button, { requireExplicitArea: true })).toBe(false);
  });
});
