import Wordmark from "../components/Wordmark";
import MessageSkeleton from "../chat/MessageSkeleton";
import "../pages/call.css";
import "../chat/chat.css";
import "./spaces.css";

/** Also rendered on the server while the client-only route boots. */
export default function SpacesLoading() {
  return (
    <main className="call-page" aria-busy="true">
      <header className="call-header">
        <Wordmark />
      </header>
      <section className="call-room spaces-room spaces-loading">
        <div className="space-rail" aria-hidden="true">
          <span className="spaces-skeleton-tile" />
          <span className="spaces-skeleton-tile" />
          <span className="spaces-skeleton-tile" />
        </div>
        <aside className="people-panel" aria-hidden="true">
          <div className="spaces-loading-heading">
            <span className="spaces-skeleton-line" />
          </div>
          <div className="spaces-loading-channels">
            <span className="spaces-skeleton-line" />
            <span className="spaces-skeleton-line" />
            <span className="spaces-skeleton-line" />
          </div>
          <div className="spaces-loading-account">
            <span className="spaces-skeleton-line" />
          </div>
        </aside>
        <div className="stage">
          <p className="spaces-loading-heading" role="status">
            Loading your spaces…
          </p>
          <div className="spaces-loading-messages">
            <MessageSkeleton />
          </div>
          <div className="spaces-loading-composer" aria-hidden="true">
            <span className="spaces-skeleton-input" />
          </div>
        </div>
      </section>
    </main>
  );
}
