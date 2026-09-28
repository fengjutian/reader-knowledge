import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles/global.scss";
import "./styles/overrides.scss";
createRoot(document.getElementById("root")!).render(<StrictMode><App/></StrictMode>);

