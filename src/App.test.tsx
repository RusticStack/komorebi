import { fireEvent, render, screen, within } from "@testing-library/react";
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
    expect(screen.getByRole("button", { name: "Settings" })).toBeEnabled();
    expect(screen.getByRole("searchbox")).toBeDisabled();
    expect(
      screen.queryByText("Welcome to Tauri + React"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("personal@example.com")).not.toBeInTheDocument();
    expect(screen.queryByText("Client kickoff agenda")).not.toBeInTheDocument();
  });

  it("opens accountless settings without exposing fake signature data", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    const settings = screen.getByRole("region", { name: "Settings" });

    expect(settings).toBeVisible();
    expect(screen.getByRole("heading", { name: "Signatures" })).toBeVisible();
    expect(within(settings).getByText("No accounts configured.")).toBeVisible();
    expect(screen.queryByText("Personal signature")).not.toBeInTheDocument();
  });
});
