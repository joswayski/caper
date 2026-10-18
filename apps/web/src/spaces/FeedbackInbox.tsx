import { useState } from "react";
import { Archive, ArchiveRestore, ChevronDown, Inbox, LifeBuoy } from "lucide-react";
import Avatar from "../components/Avatar";
import Tooltip from "../components/Tooltip";
import type { FeedbackReport } from "./client.ts";

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
 * The feedback team's inbox: every conversation where someone asked for the
 * team, newest first. Opening one visits it in place, at the report's thread.
 * Archived threads (shared by the whole team) are one toggle away.
 */
export default function FeedbackInbox({
  reports,
  waiting,
  more,
  loading,
  error,
  current,
  archived,
  onShowArchived,
  onArchive,
  onOpen,
  onMore,
  onRetry,
}: {
  reports: FeedbackReport[];
  /** Total waiting conversations, independent of pagination or archive view. */
  waiting: number;
  /** Older reports can be loaded. */
  more: boolean;
  loading: boolean;
  error?: string;
  /** `channelId:threadRootId` of the visit on screen. */
  current?: string;
  /** Listing archived threads instead of the inbox. */
  archived: boolean;
  onShowArchived: (archived: boolean) => void;
  onArchive: (report: FeedbackReport, archived: boolean) => void;
  onOpen: (report: FeedbackReport) => void;
  onMore: () => void;
  onRetry: () => void;
}) {
  const [open, setOpen] = useState(true);
  const [shown, setShown] = useState(SHOWN);
  const threads = feedbackThreads(reports);
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
          {archived ? "Archived feedback" : "Feedback"}
          <ChevronDown className="feedback-chevron" aria-hidden="true" />
        </button>
        {waiting > 0 && (
          <span className="feedback-waiting-count" aria-label={`${waiting} feedback conversations waiting for a reply`}>
            {waiting}
          </span>
        )}
      </div>
      {open && (
        <>
          <ul id="feedback-list">
            {visible.map((report) => {
              const key = `${report.channel.id}:${report.threadRootId}`;
              const label = `${archived ? "Restore" : "Archive"} feedback from ${report.reporter.displayName}`;
              return (
                <li key={key}>
                  <div className="feedback-line">
                    <button
                      type="button"
                      className="channel-select direct-select feedback-select"
                      aria-current={current === key ? "page" : undefined}
                      title={`${report.reporter.displayName} in ${feedbackPlace(report)}`}
                      onClick={() => onOpen(report)}
                    >
                      <span className="direct-avatar" aria-hidden="true">
                        <Avatar person={report.reporter} name={report.reporter.displayName} />
                      </span>
                      <span className="feedback-row">
                        <span className="feedback-row-top">
                          <span>{report.reporter.displayName}</span>
                          {!archived && (
                            <small data-waiting={report.answered ? undefined : ""}>
                              {report.answered ? "Answered" : "Waiting"}
                            </small>
                          )}
                        </span>
                        <small className="feedback-row-detail">
                          {feedbackPlace(report)} — {feedbackPreview(report)}
                        </small>
                      </span>
                    </button>
                    <Tooltip content={archived ? "Restore to the inbox" : "Archive"}>
                      <button
                        type="button"
                        className="feedback-archive"
                        aria-label={label}
                        onClick={() => onArchive(report, !archived)}
                      >
                        {archived ? <ArchiveRestore aria-hidden="true" /> : <Archive aria-hidden="true" />}
                      </button>
                    </Tooltip>
                  </div>
                </li>
              );
            })}
          </ul>
          {!loading && !error && !threads.length && (
            <p className="feedback-empty">{archived ? "Nothing archived." : "No feedback yet."}</p>
          )}
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
          <button
            type="button"
            className="channel-select direct-action feedback-toggle"
            onClick={() => {
              setShown(SHOWN);
              onShowArchived(!archived);
            }}
          >
            {archived ? <Inbox aria-hidden="true" /> : <Archive aria-hidden="true" />}
            <span>{archived ? "Back to feedback" : "Show archived"}</span>
          </button>
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
