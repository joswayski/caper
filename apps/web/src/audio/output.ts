type SinkElement = { setSinkId?: (id: string) => Promise<void> };

/** Route a media element to `output`, falling back to the system default.
 * iOS WebKit can reject even the device it listed as default; audio still plays
 * there, so only a failure of the system default itself counts as unavailable. */
export async function routeOutput(element: SinkElement, output: string): Promise<boolean> {
  if (!element.setSinkId) return true;
  try {
    await element.setSinkId(output);
    return true;
  } catch {
    if (!output) return false;
  }
  try {
    await element.setSinkId("");
    return true;
  } catch {
    return false;
  }
}
