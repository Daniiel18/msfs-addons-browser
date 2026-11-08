/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  // Theme toggle controlado por `html.classList`. El default es
  // dark (igual que antes); cuando el usuario activa light en
  // Settings, removemos la clase `dark` del root y los `dark:`
  // modifiers en Tailwind dejan de aplicar.
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // (Rediseño F1) Acento turquesa #35CDB2 (= brand-500). Antes era
        // verde #1ea45d; el verde "éxito" sigue viviendo en emerald-*.
        brand: {
          50:  "#eefcf9",
          100: "#d3f7ef",
          200: "#a6eedf",
          300: "#6fe0cb",
          400: "#48d4bb",
          500: "#35cdb2",
          600: "#22a893",
          700: "#1d8778",
          800: "#1c6b61",
          900: "#1a5850",
          950: "#0c3330",
        },
        // (8.4.x) Grafito neutro en lugar del gris azulado de Tailwind: los
        // `slate-*` fijos de toda la app siguen el modo oscuro nuevo sin
        // tocar cada componente (el tema claro los sobreescribe por clase en
        // globals.css). Misma luminosidad por tono, sin el tinte azul.
        slate: {
          50:  "#f5f5f7",
          100: "#ededf0",
          200: "#e0e0e4",
          300: "#c4c4c8",
          400: "#a1a1a6",
          500: "#7a7a80",
          600: "#5a5a5f",
          700: "#3a3a3c",
          800: "#2c2c2e",
          900: "#1c1c1e",
          950: "#0e0e10",
        },
        // (Rediseño F1) Tokens del sistema visual. Los valores viven como
        // variables CSS en globals.css (con variante `html.light`), así el
        // tema claro se adapta sin overrides por clase.
        sf: {
          bg:      "rgb(var(--sf-bg) / <alpha-value>)",
          surface: "rgb(var(--sf-surface) / <alpha-value>)",
          raised:  "rgb(var(--sf-raised) / <alpha-value>)",
          // (8.4.x) En oscuro el borde es blanco translúcido (como Bob):
          // `--sf-border-alpha` multiplica la opacidad pedida (/60, /70…).
          border:  "rgb(var(--sf-border) / calc(<alpha-value> * var(--sf-border-alpha, 1)))",
          // (8.4.x) Barra lateral y barra superior (#232325 en oscuro).
          sidebar: "rgb(var(--sf-sidebar) / <alpha-value>)",
          text:    "rgb(var(--sf-text) / <alpha-value>)",
          muted:   "rgb(var(--sf-muted) / <alpha-value>)",
          accent:  "rgb(var(--sf-accent) / <alpha-value>)",
          warning: "rgb(var(--sf-warning) / <alpha-value>)",
        },
      },
      fontFamily: {
        sans: ['Inter', 'system-ui', '-apple-system', 'sans-serif'],
      },
      width: {
        "sidebar-collapsed": "68px",
        "sidebar-expanded": "232px",
      },
      animation: {
        "fade-in": "fadeIn 0.25s ease-out",
        "pulse-ring": "pulseRing 1.8s cubic-bezier(0.4, 0, 0.6, 1) infinite",
      },
      keyframes: {
        fadeIn: {
          from: { opacity: "0", transform: "translateY(6px)" },
          to:   { opacity: "1", transform: "translateY(0)" },
        },
        pulseRing: {
          "0%":   { transform: "scale(0.8)", opacity: "0.8" },
          "100%": { transform: "scale(1.8)", opacity: "0" },
        },
      },
    },
  },
  plugins: [],
};
