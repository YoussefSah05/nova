import { useState } from "react";
import type { Profile } from "@/types";
import { ProfilePicker } from "@/features/profile/ProfilePicker";
import { Home } from "@/routes/Home";

type View = { screen: "picker" } | { screen: "home"; profile: Profile };

export default function App() {
  const [view, setView] = useState<View>({ screen: "picker" });

  if (view.screen === "home") {
    return <Home profile={view.profile} onSwitchProfile={() => setView({ screen: "picker" })} />;
  }

  return <ProfilePicker onSelect={(profile) => setView({ screen: "home", profile })} />;
}
