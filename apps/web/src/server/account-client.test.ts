import assert from "node:assert/strict";
import { test, vi, type TestContext } from "vitest";
import {
  AccountApiError,
  CHAT_SESSION_KEY,
  getAccount,
  getAvatarHistory,
  getRememberedAccount,
  logout,
  requestEmailCode,
  selectAvatar,
  subscribeAccount,
  updateProfile,
  verifyEmailCode,
  withAccountProfile,
} from "../account/client.ts";

function mockFetch(t: TestContext, handler: (path: string, init?: RequestInit) => Response) {
  const original = globalThis.fetch;
  globalThis.fetch = async (input, init) => handler(String(input), init);
  t.onTestFinished(() => {
    globalThis.fetch = original;
  });
}

test("web account client uses cookie sessions across the complete onboarding flow", async (t) => {
  const calls: Array<[string, RequestInit | undefined]> = [];
  mockFetch(t, (path, init) => {
    calls.push([path, init]);
    if (path.endsWith("/request")) return Response.json({ challengeId: "challenge-1" }, { status: 202 });
    if (path.endsWith("/verify"))
      return Response.json({ account: { id: "public", username: null, displayName: null } });
    if (path.endsWith("/profile"))
      return Response.json({ id: "public", username: "caper_user", displayName: "Caper User" });
    if (path.endsWith("/logout")) return new Response(null, { status: 204 });
    return Response.json({ id: "public", username: "caper_user", displayName: "Caper User" });
  });

  assert.equal((await requestEmailCode("person@example.com")).challengeId, "challenge-1");
  assert.equal((await verifyEmailCode("challenge-1", "123456")).username, null);
  assert.equal(getRememberedAccount()!.username, null);
  assert.equal((await updateProfile("caper_user", "Caper User")).username, "caper_user");
  assert.equal(getRememberedAccount()!.displayName, "Caper User");
  assert.equal((await getAccount())?.displayName, "Caper User");
  await logout();
  assert.equal(getRememberedAccount(), undefined);

  assert.deepEqual(
    calls.map(([path]) => path),
    [
      "/api/auth/email/request",
      "/api/auth/email/verify",
      "/api/account/profile",
      "/api/account/me",
      "/api/auth/logout",
    ],
  );
  assert.ok(calls.every(([, init]) => init?.credentials === "same-origin"));
  assert.deepEqual(JSON.parse(String(calls[1]?.[1]?.body)), {
    challengeId: "challenge-1",
    code: "123456",
    tokenTransport: "cookie",
  });
});

test("logout forgets the chat capability minted from the account session", async (t) => {
  mockFetch(t, () => new Response(null, { status: 204 }));
  const stored = new Map([[CHAT_SESSION_KEY, '{"token":"chat"}']]);
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    removeItem: (key: string) => void stored.delete(key),
  });
  await logout();
  assert.equal(stored.has(CHAT_SESSION_KEY), false);
});

test("profile saves refresh retained member identities and win over an older account read", async (t) => {
  const old = { id: "self", username: "old_user", displayName: "Old Name" };
  const other = { ...old, id: "other" };
  const updated = { ...old, username: "new_user", displayName: "New Name" };
  let finishRead!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input) =>
    String(input).endsWith("/profile")
      ? Response.json(updated)
      : new Promise<Response>((resolve) => {
          finishRead = resolve;
        }),
  );
  let displayed = [old, other];
  const unsubscribe = subscribeAccount(() => {
    displayed = [old, other].map((person) => withAccountProfile(person, getRememberedAccount()));
  });
  t.onTestFinished(unsubscribe);
  const staleRead = getAccount();
  await updateProfile(updated.username, updated.displayName);
  assert.deepEqual(displayed, [updated, other]);
  finishRead(Response.json(old));
  assert.deepEqual(await staleRead, updated);
  assert.deepEqual(getRememberedAccount(), updated);
  assert.deepEqual(displayed, [updated, other]);
});

test("missing session is an ordinary signed-out state", async (t) => {
  mockFetch(t, () => Response.json({ error: "unauthorized" }, { status: 401 }));
  assert.equal(await getAccount(), null);
});

test("browsing pictures does not save; selecting sends target and expected current, and a conflict leaves the account alone", async (t) => {
  const restored = { id: "self", username: "me", displayName: "Me", avatarAsset: "third-oldest" };
  let expected: string | null = "current";
  let conflict = false;
  const before = getRememberedAccount();
  mockFetch(t, (path, init) => {
    if (path === "/api/account/avatar/history?cursor=older%2Fpicture") {
      assert.equal(init?.method, undefined);
      return Response.json({
        pictures: [{ avatarAsset: "third-oldest", avatarUrl: "https://cdn.test/third" }],
        nextCursor: null,
      });
    }
    assert.equal(path, "/api/account/avatar/select");
    assert.equal(init?.method, "POST");
    assert.deepEqual(JSON.parse(String(init?.body)), { avatarAsset: "third-oldest", expectedAvatarAsset: expected });
    return conflict ? Response.json({ error: "Picture changed" }, { status: 409 }) : Response.json(restored);
  });
  assert.equal((await getAvatarHistory("older/picture")).pictures[0].avatarAsset, "third-oldest");
  assert.deepEqual(getRememberedAccount(), before);
  assert.deepEqual(await selectAvatar("third-oldest", expected), restored);
  assert.deepEqual(getRememberedAccount(), restored);
  expected = null;
  assert.deepEqual(await selectAvatar("third-oldest", null), restored);
  conflict = true;
  await assert.rejects(
    selectAvatar("third-oldest", null),
    (error: unknown) => error instanceof AccountApiError && error.status === 409,
  );
  assert.deepEqual(getRememberedAccount(), restored);
});

test("verification errors preserve the server's remaining-attempt count", async (t) => {
  mockFetch(t, () => Response.json({ error: "invalid or expired code", attemptsRemaining: 1 }, { status: 401 }));

  await assert.rejects(
    verifyEmailCode("challenge-1", "WRONG1"),
    (error: unknown) => error instanceof AccountApiError && error.status === 401 && error.attemptsRemaining === 1,
  );
});

test("proxy error bodies do not become the error message", async (t) => {
  mockFetch(t, () => Response.json({ error: true, statusCode: 500, message: "fetch failed" }, { status: 500 }));

  await assert.rejects(
    getAccount(),
    (error: unknown) =>
      error instanceof AccountApiError && error.status === 500 && error.message === "Account request failed",
  );
});
