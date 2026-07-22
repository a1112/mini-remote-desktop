
import { createRoot } from "react-dom/client";
import App from "./app/App.tsx";
import "./styles/index.css";

function showBootstrapError(error: unknown) {
  const message = error instanceof Error ? error.stack ?? error.message : String(error);
  document.body.style.margin = "0";
  document.body.style.background = "#080a0f";
  document.body.style.color = "#f8fafc";
  document.body.innerHTML = `<pre style="box-sizing:border-box;white-space:pre-wrap;padding:16px;font:12px/1.45 Consolas,monospace;">${message.replace(
    /[&<>"']/g,
    (char) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[char] ?? char,
  )}</pre>`;
}

window.addEventListener("error", (event) => {
  showBootstrapError(event.error ?? event.message);
});
window.addEventListener("unhandledrejection", (event) => {
  showBootstrapError(event.reason);
});

try {
  createRoot(document.getElementById("root")!).render(<App />);
} catch (error) {
  showBootstrapError(error);
}
