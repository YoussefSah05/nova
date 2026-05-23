import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import App from "../App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => new Promise<never>(() => {})),
}));

describe("App", () => {
  it("renders the profile picker", () => {
    render(<App />);
    expect(screen.getByText("Who's watching?")).toBeInTheDocument();
  });
});
