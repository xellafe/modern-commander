import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useUi } from "./ui";

describe("useUi clamp", () => {
  beforeEach(() => vi.stubGlobal("window", { innerHeight: 1000 }));
  afterEach(() => vi.unstubAllGlobals());

  it("limita la larghezza della sidebar a 160-480", () => {
    useUi.getState().setSidebarWidth(50);
    expect(useUi.getState().sidebarWidth).toBe(160);
    useUi.getState().setSidebarWidth(9999);
    expect(useUi.getState().sidebarWidth).toBe(480);
    useUi.getState().setSidebarWidth(300);
    expect(useUi.getState().sidebarWidth).toBe(300);
  });

  it("limita l'altezza del pannello a 100..60% della finestra", () => {
    useUi.getState().setBottomPanelHeight(10);
    expect(useUi.getState().bottomPanelHeight).toBe(100);
    useUi.getState().setBottomPanelHeight(9999);
    expect(useUi.getState().bottomPanelHeight).toBe(600);
    useUi.getState().setBottomPanelHeight(250);
    expect(useUi.getState().bottomPanelHeight).toBe(250);
  });
});
