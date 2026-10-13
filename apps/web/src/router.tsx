import { createRouter } from "@tanstack/react-router";
import { RouteError } from "./components/StatusPage";
import { routeTree } from "./routeTree.gen";

export function getRouter() {
  return createRouter({
    routeTree,
    defaultPreload: "intent",
    scrollRestoration: true,
    defaultErrorComponent: RouteError,
  });
}
