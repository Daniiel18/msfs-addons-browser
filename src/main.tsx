import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { preloadLocale } from "./lib/i18n";
import "./styles/globals.css";

// (v3.4.0) Pre-cargar el locale SÍNCRONO antes del primer render.
//
// La DB de Tauri es async, así que sin esto el shell entero (tabs,
// splash, errores) arrancaba en "en" (default) y sólo se "corregía"
// tras el bootstrap → el usuario nunca veía la UI en su idioma real
// hasta que las stores cargaran. Ahora leemos localStorage["simfleet.locale"]
// (escrito por la store cada vez que el usuario persiste un idioma) y
// configuramos el dict activo ANTES de montar React. El bootstrap
// asíncrono sigue ocurriendo y puede sobrescribir el valor si la DB
// dice otra cosa (raro — sólo tras un cloud-sync que descargue
// preferencias de otra PC).
preloadLocale();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>
);
