import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { SettingRow, SettingsButton, SettingsCard } from "../SettingsUI";

interface ExternalModel {
  modelId: string;
  capability: string;
  availability: "missing" | "unverified" | "available" | "unavailable";
}

interface ExternalLibrary {
  libraryId: string;
  displayName: string;
  availability: "available" | "unavailable";
  observedAt: number;
  models: ExternalModel[];
}

export function ExternalModelLibraries() {
  const [libraries, setLibraries] = useState<ExternalLibrary[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);

  const refresh = useCallback(async () => {
    const current = ++request.current;
    try {
      const result = await invoke<ExternalLibrary[]>("list_external_model_libraries");
      if (current === request.current) {
        setLibraries(result);
        setError(null);
      }
    } catch {
      if (current === request.current) setError("Could not check external model libraries.");
    }
  }, []);

  useEffect(() => {
    const currentRequest = request;
    void refresh();
    const timer = window.setInterval(() => void refresh(), 15_000);
    return () => {
      currentRequest.current++;
      window.clearInterval(timer);
    };
  }, [refresh]);

  const add = async () => {
    setBusy(true);
    try {
      const id = await invoke<string | null>("add_external_model_library");
      if (id) await refresh();
    } catch {
      setError("Could not add that model folder.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsCard title="External model libraries">
      <SettingRow
        title="Add a model folder"
        description="Choose a folder containing models/whisper or models/embeddings. Nova only reads it."
      >
        <SettingsButton onClick={add} disabled={busy}>
          {busy ? "Opening…" : "Choose folder"}
        </SettingsButton>
      </SettingRow>
      {libraries.map((library) => (
        <SettingRow
          key={library.libraryId}
          title={`${library.displayName} · ${library.libraryId.slice(-8)}`}
          description={`${library.availability === "available" ? "Connected" : "Disconnected"} · ${library.models.map((model) => `${model.capability}: ${model.availability}`).join(" · ")}`}
        />
      ))}
      {error && (
        <p className="py-3 text-xs text-rose-300" role="alert">
          {error}
        </p>
      )}
      <SettingRow
        title="Refresh library status"
        description="Disconnected folders keep their model identity and return when the drive is available. Current AI jobs still use Nova's internal models."
      >
        <SettingsButton onClick={() => void refresh()}>Refresh</SettingsButton>
      </SettingRow>
    </SettingsCard>
  );
}
