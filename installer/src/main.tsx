import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { UninstallApp } from "./uninstall/UninstallApp";
import { launchMode } from "./lib/api";
import "./index.css";

// The same program installs the DAW and, started with --uninstall,
// removes it; the backend says which window this run is.
launchMode()
  .catch(() => "install" as const)
  .then((mode) => {
    ReactDOM.createRoot(document.getElementById("root")!).render(
      <React.StrictMode>{mode === "uninstall" ? <UninstallApp /> : <App />}</React.StrictMode>
    );
  });
