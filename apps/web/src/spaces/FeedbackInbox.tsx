import { useState } from "react";
import { ChevronDown, LifeBuoy } from "lucide-react";
import Avatar from "../components/Avatar";
import type { FeedbackReport } from "./client.ts";
import type { VoiceSlot } from "../pages/Call";

/** Each thread once, by its newest report: replies to all of them belong in that thread. */
export function feedbackThreads(reports: FeedbackReport[]) {
  const seen = new Set<string>();
  return reports.filter((report) => {
    const key = `${report.channel.id}:${report.threadRootId}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function feedbackPlace(report: FeedbackReport) {
  return report.space ? `#${report.channel.name} · ${report.space.name}` : `DM · ${report.channel.name}`;
}

export function feedbackPreview(report: FeedbackReport) {
  const text = report.text.trim();
  if (text) return text;
  return report.attachments === 1 ? "Sent a file" : `Sent ${report.attachments} files`;
}

const SHOWN = 6;

/**
 * The feedback team's inbox: every conversation where someone wrote @feedback,
 * newest first. Opening one visits it in place, at the report's thread.
 */
export default function FeedbackInbox({
  reports,
  more,
  loading,
  error,
  current,
  voice,
  onOpen,
  onMore,
  onRetry,
}: {
  reports: FeedbackReport[];
  /** Older reports can be loaded. */
  more: boolean;
  loading: boolean;
  error?: string;
  /** `channelId:threadRootId` of the visit on screen. */
  current?: string;
  /** Voice in the visited channel, shown under its row like under a channel's. */
  voice?: VoiceSlot | null;
  onOpen: (report: FeedbackReport) => void;
  onMore: () => void;
  onRetry: () => void;
}) {
  const [open, setOpen] = useState(true);
  const [shown, setShown] = useState(SHOWN);
  const threads = feedbackThreads(reports);
  const waiting = threads.filter((report) => !report.answered).length;
  const visible = threads.slice(0, shown);
  return (
    <section className="direct-section feedback-section" aria-label="Feedback">
      <div className="channel-section-heading">
        <button
          type="button"
          className="direct-section-title feedback-section-title"
          aria-expanded={open}
          aria-controls="feedback-list"
          onClick={() => setOpen(!open)}
        >
          <LifeBuoy aria-hidden="true" />
          Feedback
          <ChevronDown className="feedback-chevron" aria-hidden="true" />
        </button>
        {waiting > 0 && (
          <span className="feedback-waiting-count" aria-label={`${waiting} waiting for a reply`}>
            {waiting}
          </span>
        )}
      </div>
      {open && (
        <>
          <ul id="feedback-list">
            {visible.map((report) => {
              const key = `${report.channel.id}:${report.threadRootId}`;
              const rowVoice = current === key ? voice : undefined;
              return (
                <li key={key} data-voice={rowVoice ? "" : undefined}>
                  <div className="feedback-line">
                    <button
                      type="button"
                      className="channel-select direct-select feedback-select"
                      aria-current={current === key ? "page" : undefined}
                      title={`${report.reporter.displayName} in ${feedbackPlace(report)}`}
                      onClick={() => onOpen(report)}
                    >
                      <span className="direct-avatar" aria-hidden="true">
                        <Avatar avatarId={report.reporter.avatarId} name={report.reporter.displayName} />
                      </span>
                      <span className="feedback-row">
                        <span className="feedback-row-top">
                          <span>{report.reporter.displayName}</span>
                          <small data-waiting={report.answered ? undefined : ""}>
                            {report.answered ? "Answered" : "Waiting"}
                          </small>
                        </span>
                        <small className="feedback-row-detail">
                          {feedbackPlace(report)} — {feedbackPreview(report)}
                        </small>
                      </span>
                      {rowVoice?.timer}
                    </button>
                    {rowVoice?.summary}
                  </div>
                  {rowVoice?.list}
                </li>
              );
            })}
          </ul>
          {!loading && !error && !threads.length && <p className="feedback-empty">No feedback yet.</p>}
          {(threads.length > shown || more) && (
            <button
              type="button"
              className="channel-select direct-action"
              disabled={loading}
              onClick={() => {
                if (threads.length <= shown + SHOWN && more) onMore();
                setShown(shown + SHOWN * 3);
              }}
            >
              <ChevronDown aria-hidden="true" />
              <span>{loading ? "Loading…" : "Show more"}</span>
            </button>
          )}
          {error && (
            <p className="space-sidebar-error" role="alert">
              {error}
              <button type="button" onClick={onRetry}>
                Retry feedback
              </button>
            </p>
          )}
        </>
      )}
    </section>
  );
}
