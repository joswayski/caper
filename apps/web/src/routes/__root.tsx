import { HeadContent, Outlet, Scripts, createRootRoute } from "@tanstack/react-router";
import { useEffect, type ReactNode } from "react";
import DailyIconProvider from "../components/DailyIconProvider";
import StatusPage from "../components/StatusPage";
import { sidebarWidthScript } from "../pages/ChannelSidebar";
import { applyTheme, loadSavedTheme, themeInitScript } from "../theme/themes";
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
      { property: "og:site_name", content: "Caper" },
      { property: "og:type", content: "website" },
      { property: "og:url", content: "https://caper.chat" },
      { property: "og:title", content: "Caper - A place for your people" },
      {
        property: "og:description",
        content: "Chat with anyone, about anything.",
      },
      { name: "twitter:card", content: "summary" },
      { name: "theme-color", content: "#0b0b0c" },
      { name: "color-scheme", content: "dark" },
      { title: "Caper - A place for your people" },
    ],
    links: [
      {
        id: "caper-favicon-32",
        rel: "icon",
        type: "image/png",
        sizes: "32x32",
        href: "/icons/caper-main-v4-32.png",
      },
      {
        id: "caper-favicon-192",
        rel: "icon",
        type: "image/png",
        sizes: "192x192",
        href: "/icons/caper-main-v4-192.png",
      },
      {
        id: "caper-favicon-svg",
        rel: "icon",
        type: "image/svg+xml",
        sizes: "any",
        href: "/caper-face.svg?v=4",
      },
      {
        rel: "apple-touch-icon",
        sizes: "180x180",
        href: "/icons/caper-main-v4-180.png",
      },
      { rel: "manifest", href: "/site.webmanifest" },
      {
        rel: "preload",
        href: "/fonts/caper/Figtree.latin.woff2",
        as: "font",
        type: "font/woff2",
        crossOrigin: "anonymous",
      },
      {
        rel: "preload",
        href: "/fonts/caper/Nunito.latin.woff2",
        as: "font",
        type: "font/woff2",
        crossOrigin: "anonymous",
      },
    ],
  }),
  component: RootComponent,
  notFoundComponent: NotFound,
});

function RootComponent() {
  useEffect(() => {
    applyTheme(loadSavedTheme());
  }, []);
  return (
    <RootDocument>
      <Outlet />
    </RootDocument>
  );
}

function NotFound() {
  return (
    <StatusPage eyebrow="404" title="Nothing here.">
      <div className="status-page-actions">
        <a href="/">Return home</a>
      </div>
    </StatusPage>
  );
}

function RootDocument({ children }: Readonly<{ children: ReactNode }>) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <HeadContent />
        <style>{"html,body{background:var(--blackout,#0b0b0c)}"}</style>
        <script dangerouslySetInnerHTML={{ __html: themeInitScript }} />
        <script dangerouslySetInnerHTML={{ __html: sidebarWidthScript }} />
      </head>
      <body>
        <DailyIconProvider>{children}</DailyIconProvider>
        <Scripts />
      </body>
    </html>
  );
}
