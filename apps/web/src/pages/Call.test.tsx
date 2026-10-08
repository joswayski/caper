import { renderToStaticMarkup } from "react-dom/server";
import { expect, test, vi } from "vitest";
import Call from "./Call";

test("members start closed in the initial markup, before responsive hydration", () => {
  const membersPanel = vi.fn(() => <aside id="space-member-list">Members</aside>);
  const markup = renderToStaticMarkup(<Call membersPanel={membersPanel} />);

  expect(membersPanel).not.toHaveBeenCalled();
  expect(markup).not.toContain('id="space-member-list"');
  expect(markup).toContain('aria-label="Show member list" aria-expanded="false"');
});
