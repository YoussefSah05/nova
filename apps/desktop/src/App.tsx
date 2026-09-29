import { useState } from "react";
import type { Profile } from "@/types";
import { ProfilePicker } from "@/features/profile/ProfilePicker";
import { AppShell } from "@/app/AppShell";
import { GuideProvider } from "@/features/browse/GuideViews";

type View = { screen: "picker" } | { screen: "home"; profile: Profile };

export default function App() {
  const [view, setView] = useState<View>({ screen: "picker" });

  if (view.screen === "home") {
    return (
      <GuideProvider profileId={view.profile.id}>
        <AppShell profile={view.profile} onSwitchProfile={() => setView({ screen: "picker" })} />
      </GuideProvider>
    );
  }

  return <ProfilePicker onSelect={(profile) => setView({ screen: "home", profile })} />;
}
