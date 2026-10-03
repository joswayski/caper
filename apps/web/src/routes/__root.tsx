import {
  HeadContent,
  Outlet,
  Scripts,
  createRootRoute,
} from "@tanstack/react-router";
import type { ReactNode } from "react";
import RotatingFavicon from "../components/RotatingFavicon";
import { sidebarWidthScript } from "../pages/ChannelSidebar";
import "../index.css";

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1.0" },
      {
        name: "description",
        content: "Chat with anyone, about anything.",
      },
      { name: "theme-color", content: "#0c0d0f" },
      { name: "color-scheme", content: "dark" },
      { title: "Caper - A place for your people" },
    ],
    links: [
      {
        id: "caper-favicon-32",
        rel: "icon",
        type: "image/png",
        sizes: "32x32",
        href: "/icons/caper-main-v3-32.png",
      },
      {
        id: "caper-favicon-192",
        rel: "icon",
        type: "image/png",
        sizes: "192x192",
        href: "/icons/caper-main-v3-192.png",
      },
      {
        id: "caper-favicon-svg",
        rel: "icon",
        type: "image/svg+xml",
        sizes: "any",
        href: "/caper-face.svg?v=3",
      },
      {
        rel: "apple-touch-icon",
        sizes: "180x180",
        href: "/icons/caper-main-v3-180.png",
      },
      { rel: "manifest", href: "/site.webmanifest" },
      {
        rel: "preload",
        href: "/fonts/satoshi-400.woff2",
        as: "font",
        type: "font/woff2",
      },
      {
        rel: "preload",
        href: "/fonts/satoshi-500.woff2",
        as: "font",
        type: "font/woff2",
      },
      {
        rel: "preload",
        href: "/fonts/satoshi-700.woff2",
        as: "font",
        type: "font/woff2",
      },
      {
        rel: "preload",
        href: "/fonts/satoshi-900.woff2",
        as: "font",
        type: "font/woff2",
      },
    ],
  }),
  component: RootComponent,
  notFoundComponent: NotFound,
});

function RootComponent() {
  return <RootDocument><Outlet /></RootDocument>;
}

function NotFound() {
  return (
    <main className="not-found">
      <p>404</p>
      <h1>Nothing here.</h1>
      <a href="/">Return home</a>
    </main>
  );
}

function RootDocument({ children }: Readonly<{ children: ReactNode }>) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <HeadContent />
        <RotatingFavicon />
        <style>{"html,body{background:#0c0d0f;color-scheme:dark}"}</style>
        <script dangerouslySetInnerHTML={{ __html: sidebarWidthScript }} />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  );
}
