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
        brand: {
          50:  "#eefbf3",
          100: "#d5f5e1",
          200: "#adeac5",
          300: "#77d8a1",
          400: "#3fbf78",
          500: "#1ea45d",
          600: "#12844a",
          700: "#0f693d",
          800: "#0d5333",
          900: "#0a432b",
        },
      },
      fontFamily: {
        sans: ['Inter', 'system-ui', '-apple-system', 'sans-serif'],
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
