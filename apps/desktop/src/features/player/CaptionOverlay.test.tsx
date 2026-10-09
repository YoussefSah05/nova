import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { CaptionOverlay } from "./CaptionOverlay";
import { DEFAULT_CAPTION_STYLE } from "@/types";

afterEach(cleanup);
const source = [{ segmentId: 1, start: 0, end: 2, text: "Hello" }];
const translated = [{ segmentId: 1, start: 0, end: 2, text: "مرحبا" }];

describe("translated caption overlay", () => {
  it("keeps accessibility appearance choices in translated and dual captions", () => {
    render(
      <CaptionOverlay
        mode="dual"
        sourceCues={source}
        translatedCues={[{ ...translated[0], text: "Bonjour" }]}
        targetLang="fr"
        currentTime={1}
        captionStyle={{
          ...DEFAULT_CAPTION_STYLE,
          fontScale: 1.5,
          fontFamily: "dyslexic",
          textColor: "yellow",
          background: "box",
          bgOpacity: 0.8,
        }}
      />
    );
    expect(screen.getByText("Bonjour")).toHaveStyle({
      color: "#f5d90a",
      fontSize: "2.0625rem",
      backgroundColor: "rgba(0,0,0,0.8)",
    });
    expect(screen.getByText("Bonjour").style.fontFamily).toContain("Comic Sans");
    expect(screen.getByText("Hello")).toHaveStyle({ color: "#f5d90a", fontSize: "1.5rem" });
  });
  it("renders both committed lines with Arabic direction and typography", () => {
    render(
      <CaptionOverlay
        mode="dual"
        sourceCues={source}
        translatedCues={translated}
        targetLang="ar"
        currentTime={1}
      />
    );
    expect(screen.getByText("Hello")).toBeInTheDocument();
    expect(screen.getByText("مرحبا")).toHaveAttribute("dir", "rtl");
    expect(screen.getByText("مرحبا")).toHaveStyle({ fontFamily: "var(--font-arabic)" });
  });
  it("falls back to source while translation is missing or belongs to another segment", () => {
    const view = render(<CaptionOverlay mode="translation" sourceCues={source} currentTime={1} />);
    expect(screen.getByText("Hello")).toBeInTheDocument();
    view.rerender(
      <CaptionOverlay
        mode="dual"
        sourceCues={source}
        translatedCues={[{ ...translated[0], segmentId: 2 }]}
        currentTime={1}
      />
    );
    expect(screen.queryByText("مرحبا")).not.toBeInTheDocument();
  });
  it("does not display an expired cue or any cue while off", () => {
    const view = render(
      <CaptionOverlay
        mode="translation"
        sourceCues={source}
        translatedCues={translated}
        currentTime={2}
      />
    );
    expect(screen.queryByText("مرحبا")).not.toBeInTheDocument();
    view.rerender(
      <CaptionOverlay mode="off" sourceCues={source} translatedCues={translated} currentTime={1} />
    );
    expect(screen.queryByText("Hello")).not.toBeInTheDocument();
  });
});
