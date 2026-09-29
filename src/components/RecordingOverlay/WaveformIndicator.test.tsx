import { render } from "@testing-library/react";
import { WaveformIndicator } from "./WaveformIndicator";

test("renders five bars", () => {
  const { container } = render(<WaveformIndicator isActive />);
  expect(container.querySelectorAll(".waveform__bar")).toHaveLength(5);
});

test("toggles the active class with isActive", () => {
  const { container, rerender } = render(<WaveformIndicator isActive={false} />);
  expect(container.querySelector(".waveform")).not.toHaveClass("waveform--active");
  rerender(<WaveformIndicator isActive />);
  expect(container.querySelector(".waveform")).toHaveClass("waveform--active");
});
