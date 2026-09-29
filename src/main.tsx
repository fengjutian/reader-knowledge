import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles/global.scss";
import "./styles/overrides.scss";
createRoot(document.getElementById("root")!).render(<StrictMode><App/></StrictMode>);

requestAnimationFrame(() => requestAnimationFrame(() => {
  const boot = document.getElementById("app-boot");
  boot?.classList.add("app-boot--ready");
  window.setTimeout(() => boot?.remove(), 240);
}));

