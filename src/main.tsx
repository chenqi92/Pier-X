import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initLogger } from "./lib/logger";

// Install console + error mirroring to the shared file logger as the
// very first thing — any module-level code that runs during React's
// render (top-level errors, failed imports) then lands in pier-x.log
// instead of being lost to the DevTools buffer.
initLogger();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
