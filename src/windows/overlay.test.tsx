import { render } from "@testing-library/react";
import { vi } from "vitest";

// Stub the hydration hook so this suite never hits real IPC (mirrors
// windows/prefs.test.tsx). The event hooks (useRecordingEvents,
// useStuckPhaseWatchdog, useLicenseEvents) are safe unmocked: they rely on
// useTauriEvent, whose `listen` is neutralized globally in setupTests.ts.
vi.mock("../hooks/useAppBootstrap", () => ({
  useAppBootstrap: vi.fn(() => ({ ready: true })),
}));

import { OverlayRoot } from "./overlay";

test("renders without crashing and wires the recording pipeline at idle", () => {
  // At the default (idle) recording-store phase, RecordingOverlay renders
  // nothing — this asserts OverlayRoot mounts its hooks and the overlay
  // component cleanly without throwing.
  const { container } = render(<OverlayRoot />);
  expect(container).toBeEmptyDOMElement();
});
