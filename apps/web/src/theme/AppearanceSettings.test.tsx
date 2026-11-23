// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, test, vi } from "vitest";
import AppearanceSettings from "./AppearanceSettings";
import { OBSIDIAN_THEME, PAPER_THEME, THEME_STORAGE_KEY, loadSavedTheme } from "./themes";

afterEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute("data-appearance");
  document.documentElement.removeAttribute("data-theme-id");
});

test("renders only Obsidian and Paper clickable preset cards and switches themes live", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);

  try {
    await act(async () => {
      root.render(<AppearanceSettings />);
    });

    const buttons = [...host.querySelectorAll("button.appearance-card")];
    expect(buttons).toHaveLength(2);
    expect(buttons[0].textContent).toContain("Obsidian");
    expect(buttons[0].getAttribute("aria-pressed")).toBe("true");
    expect(buttons[1].textContent).toContain("Paper");
    expect(buttons[1].getAttribute("aria-pressed")).toBe("false");
    expect(host.querySelector('input[type="color"]')).toBeNull();
    expect(host.querySelector("textarea")).toBeNull();
    expect(host.querySelector("select")).toBeNull();

    await act(async () => {
      (buttons[1] as HTMLButtonElement).click();
    });

    expect(buttons[0].getAttribute("aria-pressed")).toBe("false");
    expect(buttons[1].getAttribute("aria-pressed")).toBe("true");
    expect(document.documentElement.dataset.themeId).toBe("paper");
    expect(document.documentElement.dataset.appearance).toBe("light");
    expect(loadSavedTheme().id).toBe(PAPER_THEME.id);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

test("normalizes legacy saved custom themes to Obsidian or Paper by appearance", () => {
  localStorage.setItem(
    THEME_STORAGE_KEY,
    JSON.stringify({
      custom: { ...OBSIDIAN_THEME, id: "asdsd", name: "asdsd", appearance: "dark" },
    }),
  );
  expect(loadSavedTheme().id).toBe(OBSIDIAN_THEME.id);

  localStorage.setItem(
    THEME_STORAGE_KEY,
    JSON.stringify({
      custom: { ...PAPER_THEME, id: "sunlight", name: "Sunlight", appearance: "light" },
    }),
  );
  expect(loadSavedTheme().id).toBe(PAPER_THEME.id);
});
