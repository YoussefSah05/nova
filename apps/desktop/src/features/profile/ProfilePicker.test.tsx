import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Profile, ProfileOverview } from "@/types";
import { ProfilePicker } from "./ProfilePicker";
import { greeting } from "./launch";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

const ada: Profile = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "Ada Lovelace",
  avatarColor: "#e0a33e",
  createdAt: 1,
  lastUsedAt: 100,
};
const max: Profile = {
  id: "22222222-2222-4222-8222-222222222222",
  name: "Max",
  avatarColor: "#4fbf8f",
  createdAt: 2,
  lastUsedAt: null,
};
const overview: Record<string, ProfileOverview> = {
  [ada.id]: {
    accentColor: "amber",
    sourceCount: 2,
    itemCount: 28_600,
    continueCount: 3,
    lastTitle: "Synthetic Film",
  },
  [max.id]: {
    accentColor: "teal",
    sourceCount: 0,
    itemCount: 0,
    continueCount: 0,
    lastTitle: null,
  },
};

let profiles: Profile[] = [];
let select: (id: string) => Promise<Profile> = async (id) =>
  profiles.find((p) => p.id === id) as Profile;

beforeEach(() => {
  vi.clearAllMocks();
  profiles = [max, ada];
  select = async (id) => profiles.find((p) => p.id === id) as Profile;
  mocks.invoke.mockImplementation(async (command: string, args?: Record<string, string>) => {
    switch (command) {
      case "list_profiles":
        return profiles;
      case "get_profile_overview":
        return overview[args!.profileId];
      case "get_model_status":
        return { installed: true, modelId: "whisper", availability: "available" };
      case "get_enrich_model_status":
        return { installed: false, modelId: "embedding", availability: "missing" };
      case "select_profile":
        return select(args!.id);
      case "inspect_profile_recovery":
        return "unsupported";
      default:
        throw new Error(`unexpected ${command}`);
    }
  });
});

function mount(onSelect = vi.fn()) {
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <ProfilePicker onSelect={onSelect} />
    </QueryClientProvider>
  );
  return onSelect;
}

describe("ProfilePicker", () => {
  it("greets by time of day", () => {
    expect(greeting(8)).toBe("Good morning");
    expect(greeting(14)).toBe("Good afternoon");
    expect(greeting(21)).toBe("Good evening");
    expect(greeting(2)).toBe("Good evening");
  });

  it("lists profiles by recency with their own progress and library", async () => {
    mount();
    const cards = await screen.findAllByRole("button", { name: /Ada Lovelace|Max/ });
    expect(cards[0]).toHaveTextContent("Ada Lovelace");
    expect(await screen.findByText("Synthetic Film")).toBeInTheDocument();
    expect(screen.getByText("28.6K titles · 2 playlists")).toBeInTheDocument();
    expect(screen.getByText("New profile")).toBeInTheDocument();
    expect(await screen.findByText("Nothing in progress")).toBeInTheDocument();
    expect(screen.getByText("No playlists yet")).toBeInTheDocument();
  });

  it("reports only real on-device intelligence and marks planned capability", async () => {
    mount();
    const strip = screen.getByRole("region", { name: "On-device intelligence" });
    const item = (label: string) => within(strip).getByText(label).closest("li")!;
    await waitFor(() =>
      expect(item("Dialogue transcription")).toHaveAttribute("data-state", "ready")
    );
    await waitFor(() => expect(item("Semantic search")).toHaveAttribute("data-state", "missing"));
    expect(item("Semantic search")).toHaveTextContent("Not installed");
    expect(item("Ask Nova")).toHaveTextContent("Coming with intelligence");
  });

  it("offers a first-run welcome when there are no profiles", async () => {
    profiles = [];
    mount();
    expect(await screen.findByRole("heading", { name: "Welcome to Nova" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Create your profile" }));
    expect(await screen.findByText("New Profile")).toBeInTheDocument();
  });

  it("shows the profile opening and blocks other choices until it resolves", async () => {
    let finish!: (profile: Profile) => void;
    select = () => new Promise((resolve) => (finish = resolve));
    const onSelect = mount();
    const card = await screen.findByRole("button", { name: /Ada Lovelace/ });
    await userEvent.click(card);
    expect(card).toHaveAttribute("aria-busy", "true");
    expect(screen.getByText("Opening…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add profile" })).toBeDisabled();
    finish(ada);
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith(ada));
  });

  it("still offers recovery when a profile fails to open", async () => {
    select = async () => {
      throw new Error("migration refused");
    };
    const onSelect = mount();
    await userEvent.click(await screen.findByRole("button", { name: /Ada Lovelace/ }));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("exposes edit and delete as their own buttons in manage mode", async () => {
    mount();
    await screen.findByRole("button", { name: /Ada Lovelace/ });
    await userEvent.click(screen.getByRole("button", { name: "Manage profiles" }));
    expect(screen.getByRole("button", { name: "Edit Ada Lovelace" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete Max" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add profile" })).not.toBeInTheDocument();
  });

  it("never reports a failed model read as not installed", async () => {
    const base = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(async (command: string, args?: Record<string, string>) => {
      if (command === "get_enrich_model_status") throw new Error("registry unavailable");
      return base(command, args);
    });
    mount();
    const strip = screen.getByRole("region", { name: "On-device intelligence" });
    const search = within(strip).getByText("Semantic search").closest("li")!;
    await waitFor(() => expect(search).toHaveAttribute("data-state", "unknown"));
    expect(search).toHaveTextContent("Couldn’t check");
  });

  it("retries a failed profile load instead of hiding existing profiles", async () => {
    const base = mocks.invoke.getMockImplementation()!;
    let fail = true;
    mocks.invoke.mockImplementation(async (command: string, args?: Record<string, string>) => {
      if (command === "list_profiles" && fail) throw new Error("app.db busy");
      return base(command, args);
    });
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Try again" }));
    fail = false;
    await userEvent.click(await screen.findByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("button", { name: /Ada Lovelace/ })).toBeInTheDocument();
  });

  it("leaves manage mode after deleting the last profile, and reports failed deletes", async () => {
    profiles = [ada];
    const base = mocks.invoke.getMockImplementation()!;
    let refuse = true;
    mocks.invoke.mockImplementation(async (command: string, args?: Record<string, string>) => {
      if (command === "delete_profile") {
        if (refuse) throw new Error("locked");
        return null;
      }
      return base(command, args);
    });
    mount();
    await screen.findByRole("button", { name: /Ada Lovelace/ });
    await userEvent.click(screen.getByRole("button", { name: "Manage profiles" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete Ada Lovelace" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("couldn’t be removed");
    refuse = false;
    await userEvent.click(screen.getByRole("button", { name: "Delete Ada Lovelace" }));
    expect(await screen.findByRole("heading", { name: "Welcome to Nova" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Done" })).not.toBeInTheDocument();
  });

  it("takes other profiles out of the tab order while one opens", async () => {
    select = () => new Promise(() => {});
    mount();
    await userEvent.click(await screen.findByRole("button", { name: /Ada Lovelace/ }));
    const max = screen.getByText("Max").closest("[inert]");
    expect(max).not.toBeNull();
    expect(screen.getByRole("button", { name: "Manage profiles" })).toBeDisabled();
  });
});
