import { FileText, Film, Image } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { dailyIconUrl } from "../components/daily-icon";
import { dropPlan, dropPose, dropText } from "./drop.ts";

/** Covers the conversation while files are dragged over it: a random Caper
 * character hops in front of three file cards that drift apart behind it.
 * Each open picks a new character and a new seed for the motion. */
export default function DropOverlay({
  channelName,
  direct,
  attached,
}: {
  channelName: string;
  direct: boolean;
  attached: number;
}) {
  const { title, detail } = dropText(channelName, direct, attached);
  const [{ character, plan }] = useState(() => ({
    character: Math.floor(Math.random() * 800),
    plan: dropPlan(Math.floor(Math.random() * 2 ** 32)),
  }));
  const cards = useRef<(HTMLSpanElement | null)[]>([]);
  const caper = useRef<HTMLSpanElement>(null);

  // Written straight to the elements every frame, so the drift stays smooth
  // without re-rendering React.
  useLayoutEffect(() => {
    const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const start = performance.now();
    let frame = 0;
    const draw = (now: number) => {
      const pose = dropPose(plan, (now - start) / 1000, still);
      pose.cards.forEach(({ x, y, turn, opacity }, index) => {
        const card = cards.current[index];
        if (!card) return;
        card.style.transform = `translate3d(${x}px, ${y}px, 0) rotate(${turn}deg)`;
        card.style.opacity = `${opacity}`;
      });
      const { y, scaleX, scaleY, turn, opacity } = pose.character;
      if (caper.current) {
        caper.current.style.transform = `translate3d(0, ${y}px, 0) rotate(${turn}deg) scale(${scaleX}, ${scaleY})`;
        caper.current.style.opacity = `${opacity}`;
      }
      if (!still) frame = requestAnimationFrame(draw);
    };
    draw(start);
    return () => cancelAnimationFrame(frame);
  }, [plan]);

  return (
    <div className="chat-drop" role="status">
      <div className="chat-drop-card">
        <div className="chat-drop-art" aria-hidden="true">
          {[Image, FileText, Film].map((Icon, position) => (
            <span
              key={position}
              ref={(node) => {
                cards.current[position] = node;
              }}
              className="chat-drop-file"
              data-position={position}
            >
              <span>
                <Icon size={20} strokeWidth={2} />
              </span>
            </span>
          ))}
          <span ref={caper} className="chat-drop-character">
            <img src={dailyIconUrl(character)} alt="" width="256" height="256" draggable={false} />
          </span>
        </div>
        <strong>{title}</strong>
        <span>{detail}</span>
      </div>
    </div>
  );
}
