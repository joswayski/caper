import assert from "node:assert/strict";
import { test } from "node:test";
import {
  SpacesApiError,
  channelNameError,
  createChannel,
  createSpace,
  deleteSpace,
  spaceNameError,
} from "../spaces/client.ts";

test("space and channel names enforce the browser-visible API rules", () => {
  assert.equal(spaceNameError("  Studio  "), undefined);
  assert.match(spaceNameError("\u0000")!, /control/);
  assert.match(spaceNameError("x".repeat(81))!, /80/);
  assert.equal(channelNameError("launch-plans"), undefined);
  for (const name of ["Launch-plans", "launch--plans", "launch-2", "launch-"]) {
    assert.match(channelNameError(name)!, /lowercase/);
  }
});

test("CRUD sends only the specified payload and preserves server error details", async (t) => {
  const requests: Array<{ path: string; init?: RequestInit }> = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    requests.push({ path: String(input), init });
    if (String(input) === "/api/spaces") return Response.json({ id: "space1234567", name: "Studio", ownerId: "owner1234567" });
    if (init?.method === "DELETE") return Response.json({ error: "The default space cannot be deleted." }, { status: 409 });
    return Response.json({ id: "chanl1234567", spaceId: "space1234567", name: "launch-plans", private: true });
  });

  await createSpace("  Studio  ");
  await createChannel("space1234567", "launch-plans", true);
  await assert.rejects(deleteSpace("space1234567"), (error) => {
    assert.ok(error instanceof SpacesApiError);
    assert.equal(error.status, 409);
    assert.equal(error.message, "The default space cannot be deleted.");
    return true;
  });
  assert.deepEqual(JSON.parse(String(requests[0].init?.body)), { name: "Studio" });
  assert.deepEqual(JSON.parse(String(requests[1].init?.body)), { name: "launch-plans", private: true });
  assert.equal(requests[1].path, "/api/spaces/space1234567/channels");
  assert.equal(requests[2].path, "/api/spaces/space1234567");
  assert.ok(requests.every(({ init }) => init?.credentials === "same-origin"));
});
