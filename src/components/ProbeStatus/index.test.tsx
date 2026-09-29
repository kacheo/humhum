import { render, screen } from "@testing-library/react";
import { ProbeStatus } from ".";

test("renders nothing when result is null", () => {
  const { container } = render(<ProbeStatus result={null} />);
  expect(container).toBeEmptyDOMElement();
});

test("defaults to role=status and the non-error class on success", () => {
  render(<ProbeStatus result={{ ok: true, msg: "all good" }} />);
  const el = screen.getByRole("status");
  expect(el).toHaveTextContent("all good");
  expect(el).toHaveClass("probe-status");
  expect(el).not.toHaveClass("probe-status--error");
});

test("applies the error class on failure, still under role=status by default", () => {
  render(<ProbeStatus result={{ ok: false, msg: "nope" }} />);
  const el = screen.getByRole("status");
  expect(el).toHaveTextContent("nope");
  expect(el).toHaveClass("probe-status", "probe-status--error");
});

test("accepts a role=alert override", () => {
  render(<ProbeStatus result={{ ok: false, msg: "activation_failed" }} role="alert" />);
  expect(screen.getByRole("alert")).toHaveTextContent("activation_failed");
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
