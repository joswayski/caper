import { createFileRoute } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeader, setResponseHeader } from "@tanstack/react-start/server";
import { getInitialAccount } from "../account/server";
import { getPublicChannel } from "../spaces/server";
import { detectDownloadPlatform } from "../downloads";
import Home from "../pages/Home";

const getDownloadPlatform = createServerFn({ method: "GET" }).handler(() => {
  setResponseHeader("Vary", "User-Agent, Sec-CH-UA-Platform, Sec-CH-UA-Mobile");
  setResponseHeader("Accept-CH", "Sec-CH-UA-Platform, Sec-CH-UA-Mobile");
  setResponseHeader("Cache-Control", "private, no-store");
  return detectDownloadPlatform({
    userAgent: getRequestHeader("user-agent") ?? "",
    platform: getRequestHeader("sec-ch-ua-platform") ?? "",
    mobile: getRequestHeader("sec-ch-ua-mobile") === "?1",
  });
});

export const Route = createFileRoute("/")({
  loader: async () => {
    const [account, history, downloadPlatform] = await Promise.all([getInitialAccount(), getPublicChannel(), getDownloadPlatform()]);
    return { account, history, downloadPlatform, initialNow: Date.now(), latestChanges: __LATEST_CHANGES__ };
  },
  component: HomeRoute,
});

function HomeRoute() {
  return <Home {...Route.useLoaderData()} />;
}
