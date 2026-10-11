/** Neutral placeholders, never fictional messages or interactive controls. */
export default function MessageSkeleton({ label = "Loading messages", rows = 5 }: { label?: string; rows?: number }) {
  return (
    <div className="chat-thread-skeleton" role="status" aria-label={label}>
      {Array.from({ length: rows }, (_, index) => (
        <div className="chat-thread-skeleton-row" key={index} aria-hidden="true">
          <span className="chat-thread-skeleton-avatar" />
          <div>
            <span className="chat-thread-skeleton-name" />
            <span className="chat-thread-skeleton-text" />
          </div>
        </div>
      ))}
    </div>
  );
}
