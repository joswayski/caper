import { FileText, Film, Image } from "lucide-react";
import DailyCaper from "../components/DailyCaper";
import { dropText } from "./drop.ts";

/** Covers the conversation while files are dragged over it: today's Caper
 * character pops up in front of three file cards that fan out behind it. */
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
  return (
    <div className="chat-drop" role="status">
      <div className="chat-drop-card">
        <div className="chat-drop-art" aria-hidden="true">
          {[Image, FileText, Film].map((Icon, position) => (
            <span key={position} className="chat-drop-file" data-position={position}>
              <span>
                <Icon size={20} strokeWidth={2} />
              </span>
            </span>
          ))}
          <span className="chat-drop-character">
            <DailyCaper />
          </span>
        </div>
        <strong>{title}</strong>
        <span>{detail}</span>
      </div>
    </div>
  );
}
