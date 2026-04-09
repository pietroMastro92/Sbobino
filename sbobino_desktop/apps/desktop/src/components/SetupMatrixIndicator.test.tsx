import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { SetupMatrixIndicator } from "./SetupMatrixIndicator";

describe("SetupMatrixIndicator", () => {
  it("renders an accessible matrix indicator with the expected cells", () => {
    const { container } = render(
      <SetupMatrixIndicator
        progress={42}
        size={72}
        ariaLabel="Preparing local setup"
      />,
    );

    expect(screen.getByRole("img", { name: "Preparing local setup" })).toHaveStyle({
      width: "72px",
      height: "72px",
    });
    expect(container.querySelectorAll(".startup-matrix-indicator__track")).toHaveLength(49);
    expect(container.querySelectorAll(".startup-matrix-indicator__active")).toHaveLength(49);
  });
});
