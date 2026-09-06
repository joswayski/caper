import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../../../../shared/design.css";
import "./styles.css";

function App() {
  return (
    <main className="desktop-shell">
      <div className="desktop-mark">caper<span /></div>
      <section>
        <p>EARLY BUILD</p>
        <h1>Good company is on the way.</h1>
        <span>The desktop foundation is ready for Caper’s first conversations.</span>
      </section>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode><App /></StrictMode>,
);
