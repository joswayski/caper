import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import FeedbackInbox from "./FeedbackInbox";

test("feedback navigation shows the server total even with no loaded reports or while viewing archives", () => {
  for (const archived of [false, true]) {
    for (const waiting of [0, 137]) {
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
      expect(markup.includes('aria-label="137 feedback conversations waiting for a reply"')).toBe(waiting > 0);
      expect(markup.includes("feedback-waiting-count")).toBe(waiting > 0);
    }
  }
});
