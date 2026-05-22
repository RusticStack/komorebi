import { render, screen } from "@testing-library/react";
import App from "./App";

describe("App", () => {
  it("renders the accountless mail client shell without fake mailbox data", () => {
    render(<App />);

    expect(
      screen.getByRole("heading", { name: "Komorebi" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Folders" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Messages" })).toBeVisible();
    expect(
      screen.getByRole("region", { name: "Selected message" }),
    ).toBeVisible();
    expect(screen.getByText("No accounts configured.")).toBeVisible();
    expect(screen.getByRole("region", { name: "Empty inbox" })).toBeVisible();
    expect(
      screen.getByRole("region", { name: "No selected message" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Compose" }),
    ).toBeDisabled();
    expect(screen.getByRole("searchbox")).toBeDisabled();
    expect(
      screen.queryByText("Welcome to Tauri + React"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("personal@example.com")).not.toBeInTheDocument();
    expect(screen.queryByText("Client kickoff agenda")).not.toBeInTheDocument();
  });
});
