import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, it, expect, vi } from "vitest";
import App from "../App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => new Promise<never>(() => {})),
}));

describe("App", () => {
  it("launches into the profile picker", () => {
    render(
      <QueryClientProvider client={new QueryClient()}>
        <App />
      </QueryClientProvider>
    );
    expect(screen.getByRole("img", { name: "Nova" })).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "Loading profiles" })).toBeInTheDocument();
  });
});
