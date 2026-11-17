import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import FeedbackInbox from "./FeedbackInbox";

test("feedback navigation shows the server total even with no loaded reports or while viewing archives", () => {
  for (const archived of [false, true]) {
    for (const waiting of [0, 1, 137]) {
      const markup = renderToStaticMarkup(
        <FeedbackInbox
          reports={[]}
          waiting={waiting}
          more={false}
          loading={false}
          archived={archived}
          onShowArchived={() => {}}
          onArchive={() => {}}
          onOpen={() => {}}
          onMore={() => {}}
          onRetry={() => {}}
        />,
      );
      const label =
        waiting === 1
          ? 'aria-label="1 feedback conversation waiting for a reply"'
          : `aria-label="${waiting} feedback conversations waiting for a reply"`;
      expect(markup.includes(label)).toBe(waiting > 0);
      expect(markup.includes("feedback-waiting-count")).toBe(waiting > 0);
    }
  }
});
