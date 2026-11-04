// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { createMemoryHistory, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { expect, test, vi } from "vitest";
import { Route } from "../routes/__root";
import DailyCaper from "./DailyCaper";

test("daily illustrations rotate without changing favicons across space navigation", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.setItem("caper.daily-icon.v1", JSON.stringify({ day: Math.floor(Date.now() / 86_400_000), index: 42 }));
  const space = createRoute({ getParentRoute: () => Route, path: "/spaces", component: () => <DailyCaper /> });
  const router = createRouter({
    routeTree: Route.addChildren([space]),
    history: createMemoryHistory({ initialEntries: ["/spaces?space=first"] }),
  });
  const root = createRoot(document);
  try {
    await act(async () => {
      await router.load();
      root.render(<RouterProvider router={router} />);
    });
    const icons = () => [...document.querySelectorAll('link[rel="icon"]')].map((link) => link.getAttribute("href"));
    const expected = ["/icons/caper-main-v3-32.png", "/icons/caper-main-v3-192.png", "/caper-face.svg?v=3"];
    expect(icons()).toEqual(expected);
    expect(document.querySelector("img")?.getAttribute("src")).toBe("/images/branding/v1/42.svg");
    const changes: (string | null)[][] = [];
    const observer = new MutationObserver(() => changes.push(icons()));
    observer.observe(document.head, { childList: true, subtree: true, attributes: true });
    try {
      await act(async () => {
        await router.navigate({ href: "/spaces?space=second" });
      });
      localStorage.setItem(
        "caper.daily-icon.v1",
        JSON.stringify({ day: Math.floor(Date.now() / 86_400_000), index: 73 }),
      );
      await act(async () => window.dispatchEvent(new Event("storage")));
      expect(document.querySelector("img")?.getAttribute("src")).toBe("/images/branding/v1/73.svg");
      expect(icons()).toEqual(expected);
      for (const observed of changes) expect(observed).toEqual(expected);
    } finally {
      observer.disconnect();
    }
  } finally {
    await act(async () => root.unmount());
    localStorage.clear();
  }
});
