import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { PersonalizationSettingsPanel } from "./PersonalizationSettingsPanel";

describe("PersonalizationSettingsPanel", () => {
  it("creates an explicit correction entry", async () => {
    const onUpsert = vi.fn().mockResolvedValue(undefined);
    render(
      <PersonalizationSettingsPanel
        settings={{ enabled: true, auto_apply_safe_corrections: false }}
        entries={[]}
        busy={false}
        onSettingsChange={vi.fn()}
        onUpsert={onUpsert}
        onDelete={vi.fn()}
        onClear={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText("Entry type"), { target: { value: "correction" } });
    fireEvent.change(screen.getByLabelText("Word or original text"), { target: { value: "Sbobbino" } });
    fireEvent.change(screen.getByLabelText("Replacement text"), { target: { value: "Sbobino" } });
    fireEvent.click(screen.getByRole("button", { name: "Add entry" }));

    expect(onUpsert).toHaveBeenCalledWith(expect.objectContaining({
      kind: "correction",
      source_text: "Sbobbino",
      replacement_text: "Sbobino",
      enabled: true,
    }));
  });

  it("edits and disables an existing entry without resetting its history", async () => {
    const onUpsert = vi.fn().mockResolvedValue(undefined);
    const entry = {
      id: "entry-1",
      kind: "vocabulary" as const,
      source_text: "Sbobbino",
      replacement_text: null,
      language_code: "it",
      enabled: true,
      hit_count: 4,
      created_at: "2026-08-24T00:00:00Z",
      updated_at: "2026-08-24T00:00:00Z",
    };
    render(
      <PersonalizationSettingsPanel
        settings={{ enabled: true, auto_apply_safe_corrections: false }}
        entries={[entry]}
        busy={false}
        onSettingsChange={vi.fn()}
        onUpsert={onUpsert}
        onDelete={vi.fn()}
        onClear={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit entry" }));
    fireEvent.change(screen.getByLabelText("Word or original text"), {
      target: { value: "Sbobino" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    expect(onUpsert).toHaveBeenCalledWith(expect.objectContaining({
      id: "entry-1",
      source_text: "Sbobino",
      hit_count: 4,
      created_at: "2026-08-24T00:00:00Z",
    }));

    fireEvent.click(screen.getByRole("checkbox", { name: "Entry enabled" }));
    expect(onUpsert).toHaveBeenLastCalledWith(expect.objectContaining({
      id: "entry-1",
      enabled: false,
    }));
  });
});
