const paths = {
  sliders: "M4 7h7m4 0h5M4 17h3m4 0h9M11 4v6M7 14v6",
  keyboard: "M5 6h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2Zm2 4h.01M12 10h.01M17 10h.01M8 14h8",
  sparkles: "M10 3l1.9 5.1L17 10l-5.1 1.9L10 17l-1.9-5.1L3 10l5.1-1.9L10 3Zm8 11v6m-3-3h6",
  shield: "M12 22s7-3 7-9V5l-7-3-7 3v8c0 6 7 9 7 9Zm-3-10 2 2 4-4",
  check: "m5 12 4 4L19 6",
  warning: "M12 8v4m0 4h.01M10.3 4.8 3.2 17a2 2 0 0 0 1.7 3h14.2a2 2 0 0 0 1.7-3L13.7 4.8a2 2 0 0 0-3.4 0Z",
  mic: "M9 5a3 3 0 0 1 6 0v6a3 3 0 0 1-6 0Zm-4 6a7 7 0 0 0 14 0m-7 7v4m-3 0h6",
  waveform: "M3 12h2m3-5v10m4-14v18m4-14v10m3-5h2",
  download: "M12 3v12m-4-4 4 4 4-4M4 16v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3",
  "arrow-right": "M4 12h15m-6-6 6 6-6 6",
  "arrow-left": "M20 12H5m6-6-6 6 6 6",
} as const;

export function Icon({ name, size = 18 }: { name: keyof typeof paths; size?: number }) {
  return (
    <svg aria-hidden="true" focusable="false" width={size} height={size} viewBox="0 0 24 24"
      fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round"
      className="ui-icon">
      <path d={paths[name]} />
    </svg>
  );
}
