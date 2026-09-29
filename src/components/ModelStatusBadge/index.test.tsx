import { render, screen } from "@testing-library/react";
import { ModelStatusBadge } from ".";

test("renders each status variant", () => {
  const { rerender } = render(<ModelStatusBadge status="not_downloaded" />);
  expect(screen.getByRole("status")).toHaveTextContent("Not downloaded");
  rerender(<ModelStatusBadge status="ready" />);
  expect(screen.getByRole("status")).toHaveTextContent("Ready");
  rerender(<ModelStatusBadge status={{ downloading: 42 }} />);
  expect(screen.getByRole("status")).toHaveTextContent("Downloading 42%");
  rerender(<ModelStatusBadge status={{ error: "insufficient_disk" }} />);
  expect(screen.getByRole("status")).toHaveTextContent("Error");
});
