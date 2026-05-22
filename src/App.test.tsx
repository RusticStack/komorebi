import { render, screen } from "@testing-library/react";
import App from "./App";

describe("App", () => {
  it("renders the placeholder mail client shell", () => {
    render(<App />);

    expect(
      screen.getByRole("heading", { name: "Komorebi" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Folders" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Messages" })).toBeVisible();
    expect(
      screen.getByRole("region", { name: "Selected message" }),
    ).toBeVisible();
    expect(
      screen.queryByText("Welcome to Tauri + React"),
    ).not.toBeInTheDocument();
  });
});
