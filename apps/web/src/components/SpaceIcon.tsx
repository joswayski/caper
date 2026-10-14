import type { SpaceIcon as Icon } from "../account/pictures";
import { initial } from "./Avatar";
import Picture from "./Picture";

/** A space's tile content: its initial, with the owner's icon over it once loaded. The tile must be positioned. */
export default function SpaceIcon({ space }: { space: Icon & { name: string } }) {
  return (
    <>
      <span aria-hidden="true">{initial(space.name)}</span>
      <Picture asset={space.iconAsset} url={space.iconUrl} />
    </>
  );
}
