/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    './src/**/*.{js,ts,jsx,tsx,mdx}',
  ],
  theme: {
    extend: {
      colors: {
        // DeepBook-inspired colors
        'deep-blue': '#0066FF',
        'deep-dark': '#0A0E17',
        'deep-card': '#141921',
        'deep-border': '#1E2530',
        'bid': '#00C076',
        'ask': '#FF4D4D',
      },
    },
  },
  plugins: [],
}
