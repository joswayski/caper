import assert from "node:assert/strict";
import test from "node:test";
import { AccountApiError, getAccount, getRememberedAccount, logout, requestEmailCode, updateProfile, verifyEmailCode } from "../account/client.ts";

function mockFetch(t: test.TestContext, handler: (path: string, init?: RequestInit) => Response) {
  const original = globalThis.fetch;
  globalThis.fetch = async (input, init) => handler(String(input), init);
  t.after(() => { globalThis.fetch = original; });
}

test("web account client uses cookie sessions across the complete onboarding flow", async (t) => {
  const calls: Array<[string, RequestInit | undefined]> = [];
  mockFetch(t, (path, init) => {
    calls.push([path, init]);
    if (path.endsWith("/request")) return Response.json({ challengeId: "challenge-1" }, { status: 202 });
    if (path.endsWith("/verify")) return Response.json({ account: { id: "public", username: null, displayName: null } });
    if (path.endsWith("/profile")) return Response.json({ id: "public", username: "caper_user", displayName: "Caper User" });
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

  assert.deepEqual(calls.map(([path]) => path), [
    "/api/auth/email/request",
    "/api/auth/email/verify",
    "/api/account/profile",
    "/api/account/me",
    "/api/auth/logout",
  ]);
  assert.ok(calls.every(([, init]) => init?.credentials === "same-origin"));
  assert.deepEqual(JSON.parse(String(calls[1]?.[1]?.body)), {
    challengeId: "challenge-1",
    code: "123456",
    tokenTransport: "cookie",
  });
});

test("missing session is an ordinary signed-out state", async (t) => {
  mockFetch(t, () => Response.json({ error: "unauthorized" }, { status: 401 }));
  assert.equal(await getAccount(), null);
});

test("verification errors preserve the server's remaining-attempt count", async (t) => {
  mockFetch(t, () => Response.json({ error: "invalid or expired code", attemptsRemaining: 1 }, { status: 401 }));

  await assert.rejects(
    verifyEmailCode("challenge-1", "WRONG1"),
    (error: unknown) => error instanceof AccountApiError
      && error.status === 401
      && error.attemptsRemaining === 1,
  );
});
