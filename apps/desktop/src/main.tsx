import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import { queryClient } from "./lib/queryClient";
// Self-hosted fonts (offline-first — bundled by Vite, no runtime CDN).
// Inter = body/UI; Bricolage Grotesque = display (hero / section titles).
import "@fontsource-variable/inter";
import "@fontsource-variable/bricolage-grotesque";
import "./styles/globals.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>
);
