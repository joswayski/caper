import { detectDownloadPlatform, downloads, testFlightUrl } from "../downloads";

/**
 * `caper.chat/download` (and `/mobile`) sends each visitor straight to their
 * install: the iOS TestFlight beta, the Windows installer or the Mac disk image.
 * Android's beta takes two steps and Linux offers a package choice, so those
 * visitors, and anyone unrecognized, land on the homepage, which shows them.
 */
export function downloadRedirect(request: Request): Response {
  const headers = request.headers;
  const platform = detectDownloadPlatform({
    userAgent: headers.get("user-agent") ?? "",
    platform: headers.get("sec-ch-ua-platform") ?? "",
    mobile: headers.get("sec-ch-ua-mobile") === "?1",
  });
  const vary = "User-Agent, Sec-CH-UA-Platform, Sec-CH-UA-Mobile";
  if (platform === "macos") {
    // iPadOS Safari sends a Mac user agent; only touch points, read in the browser, tell them apart.
    return new Response(macOrIpadPage(downloads.macos.url, testFlightUrl), {
      headers: { "content-type": "text/html; charset=utf-8", vary },
    });
  }
  const location = platform === "ios" ? testFlightUrl : platform === "windows" ? downloads.windows.url : null;
  return new Response(null, { status: 302, headers: { location: location ?? "/", vary } });
}

function macOrIpadPage(dmg: string, testFlight: string | null): string {
  const ipad = testFlight ?? "/";
  return `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Download Caper</title>
<script>location.replace(navigator.maxTouchPoints > 1 ? ${JSON.stringify(ipad)} : ${JSON.stringify(dmg)})</script>
<body style="margin:0;min-height:100vh;display:grid;place-content:center;gap:12px;text-align:center;background:#0B0B0C;color:#F4F3EF;font:16px/1.5 system-ui,sans-serif">
<p style="margin:0">Your Caper download is starting.</p>
<p style="margin:0;color:#9BA0A4"><a style="color:inherit" href="${dmg}">Mac (.dmg)</a> · <a style="color:inherit" href="${ipad}">iPad (TestFlight)</a></p>
</body>
</html>`;
}
