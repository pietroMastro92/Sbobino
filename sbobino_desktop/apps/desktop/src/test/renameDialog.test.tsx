import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { RenameDialog, restoreRenameFocus } from "../App";

afterEach(() => {
  vi.restoreAllMocks();
});

function renderRenameDialog(options?: { busy?: boolean }) {
  const onClose = vi.fn();
  const onConfirm = vi.fn();
  const onDraftChange = vi.fn();

  render(
    <RenameDialog
      draft="Meeting"
      busy={options?.busy ?? false}
      onDraftChange={onDraftChange}
      onConfirm={onConfirm}
      onClose={onClose}
    />,
  );

  return {
    dialog: screen.getByRole("dialog"),
    input: screen.getByRole("textbox"),
    cancel: screen.getByRole("button", { name: "Cancel" }),
    save: screen.getByRole("button", { name: /Save|Saving/ }),
    onClose,
    onConfirm,
  };
}

describe("RenameDialog", () => {
  it("cycles keyboard focus through enabled controls and handles Escape", () => {
    const { input, cancel, save, onClose } = renderRenameDialog();

    input.focus();
    fireEvent.keyDown(input, { key: "Tab" });
    expect(document.activeElement).toBe(cancel);

    fireEvent.keyDown(cancel, { key: "Tab" });
    expect(document.activeElement).toBe(save);

    fireEvent.keyDown(save, { key: "Tab" });
    expect(document.activeElement).toBe(input);

    fireEvent.keyDown(input, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("keeps Escape and Cancel inert while saving", () => {
    const { input, cancel, save, onClose } = renderRenameDialog({ busy: true });

    expect(cancel).toBeDisabled();
    expect(save).toBeDisabled();
    fireEvent.keyDown(input, { key: "Escape" });

    expect(onClose).not.toHaveBeenCalled();
  });

  it("restores focus to the exact opener after the dialog closes", () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    const requestAnimationFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback) => {
        callback(0);
        return 0;
      });

    function Harness(): JSX.Element {
      const [open, setOpen] = useState(true);
      return open ? (
        <RenameDialog
          draft="Meeting"
          busy={false}
          onDraftChange={vi.fn()}
          onConfirm={vi.fn()}
          onClose={() => {
            setOpen(false);
            restoreRenameFocus(opener);
          }}
        />
      ) : (
        <></>
      );
    }

    render(<Harness />);
    opener.blur();
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });

    expect(requestAnimationFrame).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });
  it("focuses the history row before restoring its hidden action", () => {
    const row = document.createElement("article");
    row.className = "history-item";
    const main = document.createElement("button");
    main.className = "history-main";
    const opener = document.createElement("button");
    row.append(main, opener);
    document.body.append(row);
    const focused: HTMLElement[] = [];
    main.addEventListener("focus", () => focused.push(main));
    opener.addEventListener("focus", () => focused.push(opener));
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    restoreRenameFocus(opener);
    expect(focused).toEqual([main, opener]);
    expect(document.activeElement).toBe(opener);
    row.remove();
  });

});
